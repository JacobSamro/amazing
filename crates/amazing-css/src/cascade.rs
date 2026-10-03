//! Cascade, inheritance, and computed styles over the arena DOM.
//!
//! Implemented:
//! - rule compilation: selector strings from [`crate::parser`] are parsed
//!   once up front; per css-selectors a rule whose selector list contains
//!   ANY invalid selector is dropped whole (never partially applied)
//! - cascade per property with the full key: importance → origin
//!   (UA < presentational hints < author; reversed for `!important`) → inline style beats sheets →
//!   layer → specificity → scope proximity → source order
//! - per-declaration source order and the specificity of the most-specific
//!   selector in a list that actually matches
//! - a private specified-to-computed stage for property metadata,
//!   custom-property inheritance/cycles, `var()` substitution, CSS-wide
//!   keywords, `all`, and supported shorthand expansion
//! - a minimal built-in user-agent stylesheet (display defaults for HTML,
//!   heading sizes, link color) so Phase 3 layout starts from real values
//!
//! Deferred/deviations:
//! - `<style media>` conditions are ignored
//! - HTML hints needing browsing/document state are tracked in css_TODO.md
//! - unknown properties get no initial metadata; user-origin sheets have no
//!   input seam yet

use crate::computed::{self, ComputedStyle, Rollback};
use crate::parser::{Declaration, Stylesheet, parse_declarations, parse_stylesheet};
use crate::selectors::{PseudoElement, Selector, Specificity, parse_selector};
use crate::values::ComputedValue;
use crate::media_queries::MediaEnvironment;
use amazing_dom::{Dom, NodeIdx};
use std::collections::{BTreeMap, HashMap, HashSet};

/// A compositor-sampled computed value entering the animation or transition
/// cascade origin. The CSS text is retained for `ComputedStyle::get()` while
/// `typed` is checked by the CSS value parser at insertion.
#[derive(Debug, Clone, PartialEq)]
pub struct AnimatedStyleValue {
    pub css: String,
    pub typed: ComputedValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AnimatedOrigin {
    Animation,
    Transition,
}

/// Per-element, per-longhand samples supplied at a rendering-update boundary.
#[derive(Debug, Clone, Default)]
pub struct AnimatedValues {
    values: BTreeMap<(NodeIdx, String, AnimatedOrigin), AnimatedStyleValue>,
    compositor_owned: BTreeMap<(NodeIdx, String), crate::animation::CompositorAnimation>,
}

impl AnimatedValues {
    /// Reject shorthands, unresolved variables and values invalid for a known
    /// CSS longhand. The compositor may use this for each sampled property.
    pub fn insert(&mut self, node: NodeIdx, property: &str, css: &str) -> bool {
        self.insert_with_origin(node, property, css, AnimatedOrigin::Animation)
    }

    pub fn insert_transition(&mut self, node: NodeIdx, property: &str, css: &str) -> bool {
        self.insert_with_origin(node, property, css, AnimatedOrigin::Transition)
    }

    pub fn insert_with_origin(
        &mut self,
        node: NodeIdx,
        property: &str,
        css: &str,
        origin: AnimatedOrigin,
    ) -> bool {
        let property = property.to_ascii_lowercase();
        let property = if property == "font-width" { "font-stretch".to_owned() } else { property };
        if self.compositor_owned.contains_key(&(node, property.clone())) {
            return false;
        }
        if !computed::is_supported_property(&property)
            || crate::syntax::tokenize(css).iter().any(|token| {
                matches!(token, crate::syntax::Token::Function(name) if name.eq_ignore_ascii_case("var"))
            })
            || matches!(
                css.trim().to_ascii_lowercase().as_str(),
                "inherit" | "initial" | "unset" | "revert" | "revert-layer"
            )
        {
            return false;
        }
        let Some(typed) = crate::values::parse(&property, css) else {
            return false;
        };
        self.values.insert(
            (node, property, origin),
            AnimatedStyleValue { css: css.to_owned(), typed },
        );
        true
    }

    pub fn get(&self, node: NodeIdx, property: &str) -> Option<&AnimatedStyleValue> {
        let property = property.to_ascii_lowercase();
        let property = if property == "font-width" { "font-stretch".to_owned() } else { property };
        if self.compositor_owned.contains_key(&(node, property.clone())) {
            return None;
        }
        self.values
            .get(&(node, property.clone(), AnimatedOrigin::Transition))
            .or_else(|| self.values.get(&(node, property, AnimatedOrigin::Animation)))
    }

    pub fn remove(&mut self, node: NodeIdx, property: &str, origin: AnimatedOrigin) {
        let property=property.to_ascii_lowercase();
        let property=if property=="font-width" {"font-stretch".to_owned()} else {property};
        self.values.remove(&(node, property, origin));
    }

    /// Hand the resolved animation to the compositor. CSS no longer inserts
    /// per-frame samples for this property until completion or cancellation.
    pub fn handoff(&mut self, animation: crate::animation::CompositorAnimation) -> bool {
        if !animation.valid() { return false; }
        let key = (animation.node, animation.property.as_str().to_owned());
        if self.compositor_owned.contains_key(&key) { return false; }
        self.values.remove(&(key.0, key.1.clone(), AnimatedOrigin::Animation));
        self.values.remove(&(key.0, key.1.clone(), AnimatedOrigin::Transition));
        self.compositor_owned.insert(key, animation);
        true
    }

    pub fn compositor_owned(&self, node: NodeIdx, property: &str) -> bool {
        self.compositor_owned.contains_key(&(node, property.to_ascii_lowercase()))
    }

    pub fn owned_animation(&self, node: NodeIdx, property: &str)
        -> Option<&crate::animation::CompositorAnimation> {
        self.compositor_owned.get(&(node, property.to_ascii_lowercase()))
    }

    /// Main-thread computed-style reads use the same pure sampler as vsync.
    pub fn sample_owned(&self, node: NodeIdx, property: &str, t_ms: f64)
        -> Option<crate::animation::AnimationSample> {
        let animation = self.compositor_owned.get(&(node, property.to_ascii_lowercase()))?;
        crate::animation::sample(animation, t_ms)
    }

    pub fn read_owned_style(&self, node: NodeIdx, property: &str, t_ms: f64)
        -> Option<AnimatedStyleValue> {
        let value = self.sample_owned(node, property, t_ms)?.value?;
        let css = value.to_css();
        let typed = crate::values::parse(property, &css)?;
        Some(AnimatedStyleValue { css, typed })
    }

    /// Commit a completed fill at its cascade origin and dispatch the end
    /// event through the caller's main-thread callback. Stale generations or
    /// early completion messages are ignored.
    pub fn commit_property_update(
        &mut self,
        update: crate::animation::AnimationPropertyUpdate,
        mut dispatch: impl FnMut(crate::animation::AnimationMainThreadEvent),
    ) -> bool {
        use crate::animation::{AnimationKind, AnimationMainThreadEvent, AnimationMainThreadEventKind};
        let key = (update.node, update.property.as_str().to_owned());
        let Some(animation) = self.compositor_owned.get(&key) else { return false; };
        if animation.id != update.id || animation.generation != update.generation { return false; }
        let sampled = crate::animation::sample(animation, update.time_ms);
        if !update.cancelled && !sampled.as_ref().is_some_and(|sample| sample.finished) { return false; }
        let animation = self.compositor_owned.remove(&key).expect("validated owner");
        let origin = match animation.kind {
            AnimationKind::Animation => AnimatedOrigin::Animation,
            AnimationKind::Transition => AnimatedOrigin::Transition,
        };
        if let Some(value) = sampled.and_then(|sample| sample.value) {
            let css = value.to_css();
            if let Some(typed) = crate::values::parse(key.1.as_str(), &css) {
                self.values.insert((key.0, key.1.clone(), origin), AnimatedStyleValue { css, typed });
            }
        }
        let kind = match (animation.kind, update.cancelled) {
            (AnimationKind::Animation, false) => AnimationMainThreadEventKind::AnimationEnd,
            (AnimationKind::Animation, true) => AnimationMainThreadEventKind::AnimationCancel,
            (AnimationKind::Transition, false) => AnimationMainThreadEventKind::TransitionEnd,
            (AnimationKind::Transition, true) => AnimationMainThreadEventKind::TransitionCancel,
        };
        let same_animation_pending = animation.kind == AnimationKind::Animation
            && self.compositor_owned.values().any(|other| {
                other.id == animation.id && other.node == animation.node
                    && other.generation == animation.generation
            });
        if !same_animation_pending {
            let active_duration = animation.timing.duration_ms * animation.timing.iterations;
            let elapsed_time_ms = if update.cancelled {
                (update.time_ms - animation.start_time_ms - animation.timing.delay_ms)
                    .clamp(0.0, active_duration)
            } else { active_duration };
            dispatch(AnimationMainThreadEvent {
                node: animation.node,
                generation: animation.generation,
                name: animation.name,
                kind,
                elapsed_time_ms,
            });
        }
        true
    }

    /// Any mutation that can invalidate a handoff returns ownership to the
    /// main thread and emits cancellation events there.
    pub fn cancel_for_mutation(
        &mut self, node: NodeIdx, generation: u64, t_ms: f64,
        mut dispatch: impl FnMut(crate::animation::AnimationMainThreadEvent),
    ) -> Vec<crate::animation::CompositorAnimation> {
        let handoffs: Vec<_> = self.compositor_owned.values()
            .filter(|animation| animation.node == node && animation.generation == generation)
            .cloned().collect();
        for animation in &handoffs {
            let update = crate::animation::AnimationPropertyUpdate {
                id: animation.id, node, generation, property: animation.property,
                time_ms: t_ms, cancelled: true,
            };
            self.commit_property_update(update, &mut dispatch);
        }
        handoffs
    }
}

/// Computed styles for a whole document, keyed by arena index.
#[derive(Debug, Default, Clone)]
pub struct StyledDom {
    styles: HashMap<u32, ComputedStyle>,
    sources: HashMap<u32, BTreeMap<String, ElementStyleSource>>,
    pseudo_styles: HashMap<(u32, PseudoElement), ComputedStyle>,
    matched_pseudos: HashSet<(u32, PseudoElement)>,
    pub(crate) generated: HashMap<(u32, PseudoElement), crate::generated::GeneratedContent>,
}

impl StyledDom {
    pub fn get(&self, idx: NodeIdx) -> Option<&ComputedStyle> {
        self.styles.get(&idx.0)
    }

    /// Computed style of a generated or applicable pseudo-element.
    pub fn get_pseudo(&self, idx: NodeIdx, pseudo: PseudoElement) -> Option<&ComputedStyle> {
        self.pseudo_styles.get(&(idx.0, pseudo))
    }

    /// Whether a stylesheet selector matched this element's pseudo-element.
    /// A matched selector may still produce no box (for example `content: none`).
    pub fn pseudo_matched(&self, idx: NodeIdx, pseudo: PseudoElement) -> bool {
        self.matched_pseudos.contains(&(idx.0, pseudo))
    }

    /// Resolved text/image runs and optional accessible alt text.
    pub fn generated_content(
        &self,
        idx: NodeIdx,
        pseudo: PseudoElement,
    ) -> Option<&crate::generated::GeneratedContent> {
        self.generated.get(&(idx.0, pseudo))
    }

    /// Winning origin for a computed property on this document element.
    pub fn source(&self, idx: NodeIdx, property: &str) -> Option<ElementStyleSource> {
        self.sources
            .get(&idx.0)?
            .get(&property.to_ascii_lowercase())
            .copied()
    }

    /// The properties of `idx` whose computed value differs from its parent
    /// element's — the readable "delta" view used by the CLI demo. Elements
    /// without a styled parent element report everything.
    pub fn delta_from_parent(
        &self,
        idx: NodeIdx,
        parent: Option<NodeIdx>,
    ) -> Vec<(String, String)> {
        let style = match self.get(idx) {
            Some(s) => s,
            None => return Vec::new(),
        };
        match parent.and_then(|p| self.get(p)) {
            None => style
                .iter()
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .collect(),
            Some(pstyle) => style
                .iter()
                .filter(|(k, v)| pstyle.get(k) != Some(*v))
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .collect(),
        }
    }
}

/// Computes styles for every element of `dom`, combining the built-in UA
/// sheet with the given author sheets (in order).
pub fn compute(dom: &Dom, author_sheets: &[Stylesheet]) -> StyledDom {
    compute_with_environment(dom, author_sheets, &MediaEnvironment::default())
}

/// Compute styles with the rendering viewport and media preferences supplied
/// by the driver. Viewport lengths become CSS px before inheritance and layout.
pub fn compute_with_environment(
    dom: &Dom,
    author_sheets: &[Stylesheet],
    environment: &MediaEnvironment,
) -> StyledDom {
    compute_with_optional_animations(dom, author_sheets, None, environment, None)
}

/// Computes the document with compositor samples in the animation and
/// transition origins; ordinary `compute` has identical behavior without them.
pub fn compute_with_animations(
    dom: &Dom,
    author_sheets: &[Stylesheet],
    animated: &AnimatedValues,
) -> StyledDom {
    compute_with_optional_animations(dom, author_sheets, Some(animated), &MediaEnvironment::default(), None)
}

pub fn compute_with_animations_and_environment(
    dom: &Dom,
    author_sheets: &[Stylesheet],
    animated: &AnimatedValues,
    environment: &MediaEnvironment,
) -> StyledDom {
    compute_with_optional_animations(dom, author_sheets, Some(animated), environment, None)
}

/// Compute with actual first-font metrics before elements and pseudos inherit.
/// Animation samples share this context; inherited numeric aspects stay fixed
/// even when a descendant selects a different font family.
pub fn compute_with_font_metrics(
    dom: &Dom, author_sheets: &[Stylesheet], environment: &MediaEnvironment,
    animated: Option<&AnimatedValues>, context: &dyn crate::font_metric_context::FontMetricContext,
) -> StyledDom {
    compute_with_optional_animations(dom, author_sheets, animated, environment, Some(context))
}

fn compute_with_optional_animations(
    dom: &Dom,
    author_sheets: &[Stylesheet],
    animated: Option<&AnimatedValues>,
    environment: &MediaEnvironment,
    font_metrics: Option<&dyn crate::font_metric_context::FontMetricContext>,
) -> StyledDom {
    compute_document_with_optional_animations(dom,NodeIdx(0),author_sheets,animated,environment,font_metrics)
}

/// Compute a native arena Document without cloning/remapping its node indices.
/// The driver supplies that document's active sheets and actual font context.
/// Non-document/out-of-arena roots yield an empty styled document.
pub fn compute_document_with_font_metrics(
    dom:&Dom, document:NodeIdx, author_sheets:&[Stylesheet], environment:&MediaEnvironment,
    animated:Option<&AnimatedValues>, context:&dyn crate::font_metric_context::FontMetricContext,
)->StyledDom {
    compute_document_with_optional_animations(dom,document,author_sheets,animated,environment,Some(context))
}
fn compute_document_with_optional_animations(
    dom:&Dom, document:NodeIdx, author_sheets:&[Stylesheet], animated:Option<&AnimatedValues>,
    environment:&MediaEnvironment, font_metrics:Option<&dyn crate::font_metric_context::FontMetricContext>,
)->StyledDom {
    if document.0 as usize>=dom.node_count() || !matches!(dom.get(document).kind,amazing_dom::NodeKind::Document) {return StyledDom::default();}
    // Compile UA + author rules up front; invalid selectors drop their rule.
    let ua_rules = compile_sheets(&[parse_stylesheet(UA_SHEET)], true, environment);
    let author_rules = compile_sheets(author_sheets, false, environment);

    let body_link = hints::body_link_in(dom, document);
    let pseudo_targets = PseudoElement::STYLED.map(|pseudo| {
        ua_rules
            .iter()
            .chain(author_rules.iter())
            .any(|rule| rule.targets(pseudo))
    });
    let mut out = StyledDom::default();
    style_subtree(
        dom,
        document,
        None,
        16.0,
        &ua_rules,
        &author_rules,
        body_link.as_deref(),
        &pseudo_targets,
        animated,
        environment,
        font_metrics,
        &mut out,
    );
    crate::generated::resolve_document_at(dom, document, &mut out, author_sheets, environment);
    out
}

fn style_subtree(
    dom: &Dom,
    node: NodeIdx,
    parent_style: Option<&ComputedStyle>,
    root_font_px: f32,
    ua_rules: &[CompiledRule],
    author_rules: &[CompiledRule],
    body_link: Option<&str>,
    pseudo_targets: &[bool; 5],
    animated: Option<&AnimatedValues>,
    environment: &MediaEnvironment,
    font_metrics: Option<&dyn crate::font_metric_context::FontMetricContext>,
    out: &mut StyledDom,
) {
    if dom.name(node).is_some() {
        let (mut style, sources) = compute_one(
            dom,
            node,
            parent_style,
            root_font_px,
            ua_rules,
            author_rules,
            body_link,
            None,
            animated,
            environment,
        );
        crate::font_metric_context::resolve(&mut style, node, None, font_metrics);
        let descendant_root_font_px = if dom.name(node) == Some("html") {
            style
                .get("font-size")
                .and_then(|value| value.strip_suffix("px"))
                .and_then(|value| value.parse::<f32>().ok())
                .unwrap_or(root_font_px)
        } else {
            root_font_px
        };
        for (pseudo, requested) in PseudoElement::STYLED.into_iter().zip(pseudo_targets) {
            if !*requested
                && !(pseudo == PseudoElement::Marker && style.get("display").is_some_and(crate::is_list_item_display))
            {
                continue;
            }
            let matched = ua_rules.iter().chain(author_rules.iter()).any(|rule| {
                rule.targets(pseudo) && rule.matching(dom, node, Some(pseudo)).is_some()
            });
            if matched {
                out.matched_pseudos.insert((node.0, pseudo));
            }
            if matches!(pseudo, PseudoElement::FirstLine | PseudoElement::FirstLetter) && !matched {
                continue;
            }
            let (mut pseudo_style, _) = compute_one(
                dom,
                node,
                Some(&style),
                descendant_root_font_px,
                ua_rules,
                author_rules,
                body_link,
                Some(pseudo),
                None,
                environment,
            );
            crate::font_metric_context::resolve(&mut pseudo_style, node, Some(pseudo), font_metrics);
            let has_content =
                !matches!(pseudo_style.get("content"), None | Some("normal" | "none"));
            let visible = style.get("display") != Some("none");
            let applies = match pseudo {
                PseudoElement::Before | PseudoElement::After => has_content,
                PseudoElement::Marker => style.get("display").is_some_and(crate::is_list_item_display)
                    && pseudo_style.get("content") != Some("none")
                    && (has_content || pseudo_style.get("list-style-type") != Some("none")
                        || pseudo_style.get("list-style-image") != Some("none")),
                PseudoElement::FirstLine | PseudoElement::FirstLetter => visible,
                _ => false,
            };
            if applies && pseudo_style.get("display") != Some("none") && visible {
                out.pseudo_styles.insert((node.0, pseudo), pseudo_style);
            }
        }
        for child in dom.children(node) {
            style_subtree(
                dom,
                child,
                Some(&style),
                descendant_root_font_px,
                ua_rules,
                author_rules,
                body_link,
                pseudo_targets,
                animated,
                environment,
                font_metrics,
                out,
            );
        }
        out.styles.insert(node.0, style);
        out.sources.insert(node.0, sources);
    } else {
        // Text/comment/document nodes carry no style; children of the
        // document node inherit from nothing.
        for child in dom.children(node) {
            style_subtree(
                dom,
                child,
                parent_style,
                root_font_px,
                ua_rules,
                author_rules,
                body_link,
                pseudo_targets,
                animated,
                environment,
                font_metrics,
                out,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Compilation + matching
// ---------------------------------------------------------------------------

#[path = "cascade_context.rs"]
mod context;
use context::{CompiledRule, compile_sheets};

#[path = "cascade_invalidation.rs"]
mod invalidation;
pub use invalidation::{recompute_roots, recompute_roots_with_font_metrics, AttributeStyleUpdate, CachedBoxSize, CompositorStyleDelta, MutationKind, RecalcRoots, RenderMutation, StyleDelta, StyleInvalidator, StyleMismatch};

#[path = "presentational_hints.rs"]
mod hints;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Origin {
    Ua,
    Hint,
    Author,
    Animation,
    Transition,
}

/// One declaration competing for a property during the cascade.
#[derive(Clone, Copy)]
struct Candidate<'a> {
    value: &'a str,
    base_url: Option<&'a str>,
    important: bool,
    inline: bool,
    origin: Origin,
    author_declaration: bool,
    spec: Specificity,
    order: u64,
    layer: usize,
    proximity: usize,
}

/// CSS Cascade 5 §6.1 order: transitions, important declarations, animations,
/// normal declarations; then layers, specificity and source order.
fn beats(new: &Candidate<'_>, cur: &Candidate<'_>) -> bool {
    if new.origin == Origin::Transition && cur.origin != Origin::Transition {
        return true;
    }
    if cur.origin == Origin::Transition && new.origin != Origin::Transition {
        return false;
    }
    match (new.important, cur.important) {
        (true, false) => return true,
        (false, true) => return false,
        _ => {}
    }
    if new.origin != cur.origin {
        // Normal origin order: author beats UA. Important reverses it.
        return if new.important {
            new.origin < cur.origin
        } else {
            new.origin > cur.origin
        };
    }
    if new.inline != cur.inline {
        return new.inline;
    }
    if new.layer != cur.layer {
        return if new.important {
            new.layer < cur.layer
        } else {
            new.layer > cur.layer
        };
    }
    if new.spec != cur.spec {
        return new.spec > cur.spec;
    }
    if new.proximity != cur.proximity {
        return new.proximity < cur.proximity;
    }
    new.order > cur.order
}

/// Retain all candidates until rollback has excluded the whole origin/layer.
fn winners<'a>(
    candidates: &BTreeMap<&'a str, Vec<Candidate<'a>>>,
    context: Option<&ComputedStyle>,
) -> BTreeMap<&'a str, Candidate<'a>> {
    candidates
        .iter()
        .filter_map(|(property, values)| {
            let mut values = values.clone();
            loop {
                let winner = values
                    .iter()
                    .copied()
                    .reduce(|a, b| if beats(&b, &a) { b } else { a })?;
                let substituted = context.and_then(|style| {
                    computed::substituted_rollback(style, property, winner.value)
                });
                if winner.value.trim().eq_ignore_ascii_case("revert")
                    || substituted == Some(Rollback::Origin)
                {
                    // UA revert is unset; author revert removes both importance levels.
                    if winner.origin == Origin::Ua {
                        return None;
                    }
                    // The animation origin is separate from author origin for
                    // `revert`; an author rollback can expose its value.
                    values.retain(|c| matches!(c.origin, Origin::Ua | Origin::Animation | Origin::Transition));
                } else if winner.value.trim().eq_ignore_ascii_case("revert-layer")
                    || substituted == Some(Rollback::Layer)
                {
                    values.retain(|c| {
                        if c.origin != winner.origin {
                            // The UA important-to-normal interval also spans
                            // every author declaration, not just UA layers.
                            return !(winner.origin == Origin::Ua && winner.important);
                        }
                        if winner.inline {
                            return !c.inline;
                        }
                        if c.inline {
                            return false;
                        }
                        // Important rollback removes the whole interval from
                        // its important level down through its normal level.
                        if winner.important {
                            return c.layer < winner.layer;
                        }
                        c.layer != winner.layer
                    });
                } else {
                    return Some((*property, winner));
                }
            }
        })
        .collect()
}

fn compute_one(
    dom: &Dom,
    el: NodeIdx,
    parent_style: Option<&ComputedStyle>,
    root_font_px: f32,
    ua_rules: &[CompiledRule],
    author_rules: &[CompiledRule],
    body_link: Option<&str>,
    pseudo: Option<PseudoElement>,
    animated: Option<&AnimatedValues>,
    environment: &MediaEnvironment,
) -> (ComputedStyle, BTreeMap<String, ElementStyleSource>) {
    /// One declaration after shorthand expansion, flattened with the
    /// cascade metadata inherited from its source.
    struct FlatDecl {
        prop: String,
        value: String,
        base_url: Option<String>,
        important: bool,
        inline: bool,
        origin: Origin,
        author_declaration: bool,
        spec: Specificity,
        order: u64,
        layer: usize,
        proximity: usize,
    }

    // Collect declarations from every matching rule plus the inline
    // style="" attribute into one owned pool, expanding box/border
    // shorthands so each longhand cascades independently — an author
    // `border: 5px solid` must beat a UA `border-top-style: none`
    // even though the computed map is flat.
    let mut pool: Vec<FlatDecl> = Vec::new();
    let mut source_order = 0u64;
    for (prop, value) in if pseudo.is_none() {
        hints::collect(dom, el, body_link)
    } else {
        Vec::new()
    } {
        source_order += 1;
        for (prop, value) in computed::expand_declaration(&prop, &value) {
            pool.push(FlatDecl {
                prop,
                value,
                base_url: None,
                important: false,
                inline: false,
                origin: Origin::Hint,
                author_declaration: false,
                spec: Specificity::default(),
                order: source_order,
                layer: usize::MAX,
                proximity: usize::MAX,
            });
        }
    }
    for rule in ua_rules.iter().chain(author_rules.iter()) {
        let Some((specificity, proximity)) = rule.matching(dom, el, pseudo) else {
            continue;
        };
        for decl in &rule.decls {
            source_order += 1;
            for (property, value) in computed::expand_declaration(&decl.property, &decl.value) {
                pool.push(FlatDecl {
                    prop: property,
                    value,
                    base_url: rule.base_url.clone(),
                    important: decl.important,
                    inline: false,
                    origin: if rule.ua { Origin::Ua } else { Origin::Author },
                    author_declaration: !rule.ua,
                    spec: specificity,
                    order: source_order,
                    layer: rule.layer,
                    proximity,
                });
            }
        }
    }
    // Inline style="": author origin, beats every sheet rule.
    let inline_decls = if pseudo.is_none() {
        attr_of(dom, el, "style")
            .map(parse_declarations)
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    for decl in &inline_decls {
        source_order += 1;
        for (property, value) in computed::expand_declaration(&decl.property, &decl.value) {
            pool.push(FlatDecl {
                prop: property,
                value,
                base_url: None,
                important: decl.important,
                inline: true,
                origin: Origin::Author,
                author_declaration: true,
                spec: Specificity::default(),
                order: source_order,
                layer: usize::MAX,
                proximity: usize::MAX,
            });
        }
    }
    if let Some(animated) = animated {
        for ((_, property, origin), sample) in animated
            .values
            .range((el, String::new(), AnimatedOrigin::Animation)..)
            .take_while(|((node, _, _), _)| *node == el)
        {
            source_order += 1;
            pool.push(FlatDecl {
                prop: property.clone(),
                value: sample.css.clone(),
                base_url: None,
                important: false,
                inline: false,
                origin: match origin {
                    AnimatedOrigin::Animation => Origin::Animation,
                    AnimatedOrigin::Transition => Origin::Transition,
                },
                author_declaration: false,
                spec: Specificity::default(),
                order: source_order,
                layer: usize::MAX,
                proximity: usize::MAX,
            });
        }
    }

    // Collect champions per property. Keys/values borrow from `pool`,
    // which is frozen from here on and outlives the loop below.
    let mut candidates: BTreeMap<&str, Vec<Candidate<'_>>> = BTreeMap::new();
    for d in &pool {
        let cand = Candidate {
            value: &d.value,
            base_url: d.base_url.as_deref(),
            important: d.important,
            inline: d.inline,
            origin: d.origin,
            author_declaration: d.author_declaration,
            spec: d.spec,
            order: d.order,
            layer: d.layer,
            proximity: d.proximity,
        };
        candidates.entry(&d.prop).or_default().push(cand);
    }

    let (mut style, sources) = resolve_candidate_pool(candidates, parent_style, root_font_px, dom.attrs_of(el), environment);
    let root = pseudo.is_none() && dom.get(el).parent.is_some_and(|parent|
        matches!(dom.get(parent).kind, amazing_dom::NodeKind::Document));
    if !matches!(pseudo, Some(PseudoElement::FirstLine | PseudoElement::FirstLetter)) {
        crate::display::normalize(&mut style, parent_style, root);
    }
    (style, sources)
}

fn source_map(
    selected: &BTreeMap<&str, Candidate<'_>>,
    parent: Option<&ComputedStyle>,
) -> BTreeMap<String, ElementStyleSource> {
    crate::computed::known_properties()
        .map(|property| {
            let source = if parent.is_some() && crate::computed::is_inherited_property(property) {
                ElementStyleSource::Inherited
            } else {
                ElementStyleSource::Initial
            };
            (property.to_owned(), source)
        })
        .chain(selected.iter().map(|(property, candidate)| {
            let source = if candidate.inline {
                ElementStyleSource::Inline
            } else if candidate.origin == Origin::Author {
                ElementStyleSource::Author
            } else if candidate.origin == Origin::Animation {
                ElementStyleSource::Animation
            } else if candidate.origin == Origin::Transition {
                ElementStyleSource::Transition
            } else if candidate.origin == Origin::Hint {
                ElementStyleSource::PresentationHint
            } else {
                ElementStyleSource::Initial
            };
            ((*property).to_owned(), source)
        }))
        .collect()
}

fn resolve_candidate_pool<'a>(
    mut candidates: BTreeMap<&'a str, Vec<Candidate<'a>>>,
    parent_style: Option<&ComputedStyle>,
    root_font_px: f32,
    attrs: Option<&[(String, String)]>,
    environment: &MediaEnvironment,
) -> (ComputedStyle, BTreeMap<String, ElementStyleSource>) {
    let mut champs = winners(&candidates, None);

    // Logical properties map with the element's COMPUTED axes, not the raw
    // winning tokens: `direction: inherit` and `var()` must see the inherited
    // or substituted value before their physical side is chosen.
    let provisional = computed::resolve_with_root_attrs_environment(
        parent_style,
        champs
            .iter()
            .map(|(property, candidate)| ((*property).to_owned(), candidate.value.to_owned()))
            .collect(),
        root_font_px,
        attrs,
        environment,
    );
    champs = winners(&candidates, Some(&provisional));
    let provisional = computed::resolve_with_root_attrs_environment(
        parent_style,
        champs
            .iter()
            .map(|(p, c)| ((*p).to_owned(), c.value.to_owned()))
            .collect(),
        root_font_px,
        attrs,
        environment,
    );
    let rtl = provisional
        .get("direction")
        .is_some_and(|value| value.eq_ignore_ascii_case("rtl"));
    let writing_mode = provisional.get("writing-mode").unwrap_or("horizontal-tb");
    for logical in [
        "margin-inline-start",
        "margin-inline-end",
        "margin-block-start",
        "margin-block-end",
    ] {
        if let Some(values) = candidates.get(logical).cloned() {
            let physical = physical_margin_for(logical, rtl, writing_mode);
            candidates.entry(physical).or_default().extend(values);
        }
    }
    // Logical sizing shares a cascade slot with its physical counterpart.
    // Both candidates keep their original specificity, layer, and source order.
    let vertical = !writing_mode.eq_ignore_ascii_case("horizontal-tb");
    for (logical, horizontal, vertical_name) in [
        ("contain-intrinsic-inline-size", "contain-intrinsic-width", "contain-intrinsic-height"),
        ("contain-intrinsic-block-size", "contain-intrinsic-height", "contain-intrinsic-width"),
        ("inline-size", "width", "height"),
        ("block-size", "height", "width"),
        ("min-inline-size", "min-width", "min-height"),
        ("min-block-size", "min-height", "min-width"),
        ("max-inline-size", "max-width", "max-height"),
        ("max-block-size", "max-height", "max-width"),
    ] {
        if let Some(values) = candidates.get(logical).cloned() {
            candidates
                .entry(if vertical { vertical_name } else { horizontal })
                .or_default()
                .extend(values);
        }
    }
    for (family, physicals) in [
        (
            "padding",
            [
                "padding-top",
                "padding-right",
                "padding-bottom",
                "padding-left",
            ],
        ),
        ("inset", ["top", "right", "bottom", "left"]),
    ] {
        for (axis, edge) in [
            ("inline", "start"),
            ("inline", "end"),
            ("block", "start"),
            ("block", "end"),
        ] {
            let logical = format!("{family}-{axis}-{edge}");
            if let Some(values) = candidates.get(logical.as_str()).cloned() {
                let margin = format!("margin-{axis}-{edge}");
                let index = match physical_margin_for(&margin, rtl, writing_mode) {
                    "margin-top" => 0,
                    "margin-right" => 1,
                    "margin-bottom" => 2,
                    _ => 3,
                };
                candidates
                    .entry(physicals[index])
                    .or_default()
                    .extend(values);
            }
        }
    }
    // Resolve logical border candidates against the same computed axes as margins.
    for (logical, margin, physicals) in [
        (
            "border-inline-start-width",
            "margin-inline-start",
            [
                "border-top-width",
                "border-right-width",
                "border-bottom-width",
                "border-left-width",
            ],
        ),
        (
            "border-inline-start-style",
            "margin-inline-start",
            [
                "border-top-style",
                "border-right-style",
                "border-bottom-style",
                "border-left-style",
            ],
        ),
        (
            "border-inline-start-color",
            "margin-inline-start",
            [
                "border-top-color",
                "border-right-color",
                "border-bottom-color",
                "border-left-color",
            ],
        ),
        (
            "border-inline-end-width",
            "margin-inline-end",
            [
                "border-top-width",
                "border-right-width",
                "border-bottom-width",
                "border-left-width",
            ],
        ),
        (
            "border-inline-end-style",
            "margin-inline-end",
            [
                "border-top-style",
                "border-right-style",
                "border-bottom-style",
                "border-left-style",
            ],
        ),
        (
            "border-inline-end-color",
            "margin-inline-end",
            [
                "border-top-color",
                "border-right-color",
                "border-bottom-color",
                "border-left-color",
            ],
        ),
        (
            "border-block-start-width",
            "margin-block-start",
            [
                "border-top-width",
                "border-right-width",
                "border-bottom-width",
                "border-left-width",
            ],
        ),
        (
            "border-block-start-style",
            "margin-block-start",
            [
                "border-top-style",
                "border-right-style",
                "border-bottom-style",
                "border-left-style",
            ],
        ),
        (
            "border-block-start-color",
            "margin-block-start",
            [
                "border-top-color",
                "border-right-color",
                "border-bottom-color",
                "border-left-color",
            ],
        ),
        (
            "border-block-end-width",
            "margin-block-end",
            [
                "border-top-width",
                "border-right-width",
                "border-bottom-width",
                "border-left-width",
            ],
        ),
        (
            "border-block-end-style",
            "margin-block-end",
            [
                "border-top-style",
                "border-right-style",
                "border-bottom-style",
                "border-left-style",
            ],
        ),
        (
            "border-block-end-color",
            "margin-block-end",
            [
                "border-top-color",
                "border-right-color",
                "border-bottom-color",
                "border-left-color",
            ],
        ),
    ] {
        if let Some(values) = candidates.get(logical).cloned() {
            let index = match physical_margin_for(margin, rtl, writing_mode) {
                "margin-top" => 0,
                "margin-right" => 1,
                "margin-bottom" => 2,
                _ => 3,
            };
            candidates
                .entry(physicals[index])
                .or_default()
                .extend(values);
        }
    }
    champs = winners(&candidates, Some(&provisional));

    let sources = source_map(&champs, parent_style);
    let author_properties: Vec<_> = champs
        .iter()
        .filter(|(_, candidate)| candidate.author_declaration)
        .map(|(property, _)| *property)
        .collect();
    let url_bases: Vec<_> = ["content", "list-style-image", "border-image-source"]
        .into_iter()
        .map(|property| (property, champs.get(property).copied()))
        .collect();
    let cascaded = champs
        .into_iter()
        .map(|(property, candidate)| (property.to_owned(), candidate.value.to_owned()))
        .collect();
    let mut style = computed::resolve_with_root_attrs_environment(parent_style, cascaded, root_font_px, attrs, environment);
    for (property, winner) in url_bases {
        let base = match winner {
            Some(candidate) if matches!(computed::substituted_css_wide(&style, property, candidate.value), Some(computed::CssWide::Inherit)) ||
                (computed::is_inherited_property(property) && matches!(computed::substituted_css_wide(&style, property, candidate.value), Some(computed::CssWide::Unset))) => {
                parent_style.and_then(|parent| parent.url_base(property))
            }
            Some(candidate) if matches!(computed::substituted_css_wide(&style, property, candidate.value), Some(computed::CssWide::Initial | computed::CssWide::Unset)) => None,
            Some(candidate) => candidate.base_url,
            None if property == "list-style-image" => parent_style.and_then(|parent| parent.url_base(property)),
            None => None,
        };
        style.set_url_base(property, base);
    }
    for property in author_properties {
        style.mark_author_specified(property);
    }
    for (logical, horizontal, vertical_name) in [
        ("contain-intrinsic-inline-size", "contain-intrinsic-width", "contain-intrinsic-height"),
        ("contain-intrinsic-block-size", "contain-intrinsic-height", "contain-intrinsic-width"),
        ("inline-size", "width", "height"),
        ("block-size", "height", "width"),
        ("min-inline-size", "min-width", "min-height"),
        ("min-block-size", "min-height", "min-width"),
        ("max-inline-size", "max-width", "max-height"),
        ("max-block-size", "max-height", "max-width"),
    ] {
        style.mirror_logical(logical, if vertical { vertical_name } else { horizontal });
    }
    for (family, physicals) in [
        (
            "margin",
            ["margin-top", "margin-right", "margin-bottom", "margin-left"],
        ),
        (
            "padding",
            [
                "padding-top",
                "padding-right",
                "padding-bottom",
                "padding-left",
            ],
        ),
        ("inset", ["top", "right", "bottom", "left"]),
    ] {
        for (axis, edge) in [
            ("inline", "start"),
            ("inline", "end"),
            ("block", "start"),
            ("block", "end"),
        ] {
            let logical = format!("{family}-{axis}-{edge}");
            let margin = format!("margin-{axis}-{edge}");
            let index = match physical_margin_for(&margin, rtl, writing_mode) {
                "margin-top" => 0,
                "margin-right" => 1,
                "margin-bottom" => 2,
                _ => 3,
            };
            style.mirror_logical(&logical, physicals[index]);
        }
    }
    for axis in ["inline", "block"] {
        for edge in ["start", "end"] {
            let margin = format!("margin-{axis}-{edge}");
            let side = physical_margin_for(&margin, rtl, writing_mode).strip_prefix("margin-").unwrap();
            for component in ["width", "style", "color"] {
                style.mirror_logical(&format!("border-{axis}-{edge}-{component}"),
                    &format!("border-{side}-{component}"));
            }
        }
    }
    computed::logical_border::serialize_shorthands(&mut style);
    computed::contain_intrinsic::serialize_shorthand(&mut style);
    let mut sources = sources;
    for (logical, horizontal, vertical_name) in [
        ("contain-intrinsic-inline-size", "contain-intrinsic-width", "contain-intrinsic-height"),
        ("contain-intrinsic-block-size", "contain-intrinsic-height", "contain-intrinsic-width"),
        ("inline-size", "width", "height"),
        ("block-size", "height", "width"),
        ("min-inline-size", "min-width", "min-height"),
        ("min-block-size", "min-height", "min-width"),
        ("max-inline-size", "max-width", "max-height"),
        ("max-block-size", "max-height", "max-width"),
    ] {
        if let Some(source) = sources
            .get(if vertical { vertical_name } else { horizontal })
            .copied()
        {
            sources.insert(logical.into(), source);
        }
    }
    for family in ["margin", "padding", "inset"] {
        for (axis, edge) in [
            ("inline", "start"),
            ("inline", "end"),
            ("block", "start"),
            ("block", "end"),
        ] {
            let logical = format!("{family}-{axis}-{edge}");
            let margin = format!("margin-{axis}-{edge}");
            let side = physical_margin_for(&margin, rtl, writing_mode)
                .strip_prefix("margin-")
                .unwrap();
            let physical = if family == "inset" {
                side.to_owned()
            } else {
                format!("{family}-{side}")
            };
            if let Some(source) = sources.get(&physical).copied() {
                sources.insert(logical, source);
            }
        }
    }
    (style, sources)
}

/// Origin of the declaration that supplied a standalone paint property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementStyleSource {
    Initial,
    Inherited,
    PresentationHint,
    Author,
    Inline,
    Animation,
    Transition,
}

/// A computed value within the SVG paint subset. Unsupported values retain
/// their computed CSS text so consumers can decline rendering explicitly.
#[derive(Debug, Clone, PartialEq)]
pub enum ElementStyleValue<T> {
    Supported {
        value: T,
        source: ElementStyleSource,
    },
    Unsupported {
        css: String,
        source: ElementStyleSource,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementFill {
    None,
    Color([u8; 4]),
}

/// Closed paint subset for an element parsed without a document DOM.
/// `computed` can be passed as the parent when computing a descendant.
#[derive(Debug, Clone, PartialEq)]
pub struct ElementPaintStyle {
    pub computed: ComputedStyle,
    pub color: ElementStyleValue<[u8; 4]>,
    pub fill: ElementStyleValue<ElementFill>,
    pub fill_opacity: ElementStyleValue<f32>,
    pub opacity: ElementStyleValue<f32>,
}

/// Compute paint values for a standalone element using the document cascade's
/// hint, author and inline priority and the same variable/keyword resolver.
/// Hints are always normal declarations. Only the four typed paint properties
/// are exposed; custom properties remain internal substitution state.
pub fn compute_element_paint_style(
    parent: Option<&ComputedStyle>,
    presentation_hints: &[Declaration],
    author: &[Declaration],
    inline: &[Declaration],
) -> ElementPaintStyle {
    let mut pool = Vec::new();
    for (declarations, origin, is_inline) in [
        (presentation_hints, Origin::Hint, false),
        (author, Origin::Author, false),
        (inline, Origin::Author, true),
    ] {
        for declaration in declarations {
            let normalized = if origin == Origin::Hint {
                if declaration.important {
                    continue;
                }
                crate::svg_presentation::normalize(
                    &declaration.property,
                    &declaration.value,
                    "rect",
                )
            } else {
                Some((declaration.property.clone(), declaration.value.clone()))
            };
            let Some((name, value)) = normalized else {
                continue;
            };
            for (property, value) in computed::expand_declaration(&name, &value) {
                pool.push((
                    property,
                    value,
                    origin,
                    is_inline,
                    declaration.important && origin != Origin::Hint,
                ));
            }
        }
    }
    let mut candidates: BTreeMap<&str, Vec<Candidate<'_>>> = BTreeMap::new();
    for (order, (property, value, origin, is_inline, important)) in pool.iter().enumerate() {
        candidates.entry(property).or_default().push(Candidate {
            value,
            base_url: None,
            important: *important,
            inline: *is_inline,
            origin: *origin,
            author_declaration: *origin == Origin::Author,
            spec: Specificity::default(),
            order: order as u64 + 1,
            layer: usize::MAX,
            proximity: usize::MAX,
        });
    }
    let (computed, sources) = resolve_candidate_pool(candidates, parent, 16.0, None, &MediaEnvironment::default());
    let source = |property: &str, _inherited: bool| {
        sources
            .get(property)
            .copied()
            .unwrap_or(ElementStyleSource::Initial)
    };
    let color_css = computed.get("color").unwrap_or("black");
    let color = match crate::color::parse_css_color(color_css) {
        Some(value) => ElementStyleValue::Supported {
            value,
            source: source("color", true),
        },
        None => ElementStyleValue::Unsupported {
            css: color_css.to_owned(),
            source: source("color", true),
        },
    };
    let fill_css = computed.get("fill").unwrap_or("black");
    let fill_value = if fill_css.eq_ignore_ascii_case("none") {
        Some(ElementFill::None)
    } else if fill_css.eq_ignore_ascii_case("currentcolor") {
        crate::color::parse_css_color(color_css).map(ElementFill::Color)
    } else {
        crate::color::parse_css_color(fill_css).map(ElementFill::Color)
    };
    let fill = match fill_value {
        Some(value) => ElementStyleValue::Supported {
            value,
            source: source("fill", true),
        },
        None => ElementStyleValue::Unsupported {
            css: fill_css.to_owned(),
            source: source("fill", true),
        },
    };
    let alpha = |property: &str, inherited: bool| {
        let css = computed.get(property).unwrap_or("1");
        match css.parse::<f32>() {
            Ok(value) if value.is_finite() => ElementStyleValue::Supported {
                value,
                source: source(property, inherited),
            },
            _ => ElementStyleValue::Unsupported {
                css: css.to_owned(),
                source: source(property, inherited),
            },
        }
    };
    ElementPaintStyle {
        color,
        fill,
        fill_opacity: alpha("fill-opacity", true),
        opacity: alpha("opacity", false),
        computed,
    }
}

fn physical_margin_for(logical: &str, rtl: bool, writing_mode: &str) -> &'static str {
    if writing_mode.eq_ignore_ascii_case("horizontal-tb") {
        match (logical, rtl) {
            ("margin-inline-start", false) | ("margin-inline-end", true) => "margin-left",
            ("margin-inline-start", true) | ("margin-inline-end", false) => "margin-right",
            ("margin-block-start", _) => "margin-top",
            _ => "margin-bottom",
        }
    } else {
        let rl = writing_mode.eq_ignore_ascii_case("vertical-rl")
            || writing_mode.eq_ignore_ascii_case("sideways-rl");
        match (logical, rtl, rl) {
            ("margin-inline-start", false, _) | ("margin-inline-end", true, _) => "margin-top",
            ("margin-inline-start", true, _) | ("margin-inline-end", false, _) => "margin-bottom",
            ("margin-block-start", _, true) | ("margin-block-end", _, false) => "margin-right",
            _ => "margin-left",
        }
    }
}

fn attr_of<'d>(dom: &'d Dom, el: NodeIdx, name: &str) -> Option<&'d str> {
    dom.attrs_of(el)?
        .iter()
        .find(|kv| kv.0 == name)
        .map(|(_, v)| v.as_str())
}

// ---------------------------------------------------------------------------
// Built-in user-agent stylesheet
// ---------------------------------------------------------------------------

/// Minimal standards-mode UA sheet (subset of the WHATWG rendering section).
/// Values approximate Chrome/Firefox defaults; logical properties are written
/// physically (`padding-left`) since RTL is unsupported.
const UA_SHEET: &str = r#"
html { display: block; color: black; background-color: white;
       font-family: serif; font-size: 16px; }
head, base, link, meta, script, style, template, title { display: none; }
[hidden] { display: none; }
body { display: block; margin: 8px; }

address, article, aside, blockquote, details, dialog, div, dl, fieldset,
figcaption, figure, footer, form, header, hgroup, main, menu, nav, ol, p,
pre, section, ul { display: block; }
blockquote, figure, menu, ol, p, pre, address, dl, ul { margin: 1em 0; }
ol, ul, menu { counter-reset: list-item; }
dd { display: block; margin: 0 0 0 40px; }
dt { display: block; }

h1, h2, h3, h4, h5, h6 { display: block; font-weight: bold; }
h1 { font-size: 2em; margin: 0.67em 0; }
h2 { font-size: 1.5em; margin: 0.83em 0; }
h3 { font-size: 1.17em; margin: 1em 0; }
h4 { font-size: 1em; margin: 1.33em 0; }
h5 { font-size: 0.83em; margin: 1.67em 0; }
h6 { font-size: 0.67em; margin: 2.33em 0; }

b, strong { font-weight: bolder; }
i, em, cite, var, dfn { font-style: italic; }
u, ins { text-decoration: underline; }
s, strike, del { text-decoration: line-through; }
a:link { color: #0000ee; text-decoration: underline; }
a:visited { color: #551a8b; text-decoration: underline; }
code, kbd, samp, tt { font-family: monospace; }
small { font-size: 0.83em; }
big { font-size: 1.17em; }
center { text-align: center; }
abbr[title] { text-decoration: underline dotted; }

ul, dir { padding-left: 40px; list-style-type: disc; }
ol { padding-left: 40px; list-style-type: decimal; }
li { display: list-item; }

table { display: table; border-spacing: 2px; border-collapse: separate; }
caption { display: table-caption; }
colgroup { display: table-column-group; }
col { display: table-column; }
thead { display: table-header-group; }
tbody { display: table-row-group; }
tfoot { display: table-footer-group; }
tr { display: table-row; }
td, th { display: table-cell; padding: 1px; }
th { font-weight: bold; text-align: center; }

hr { display: block; margin: 0.5em auto;
     border-width: 1px; border-style: inset; overflow: hidden; }
button, input, select { display: inline-block; }
textarea { display: inline-block; font-family: monospace; }
ruby { display: ruby; }
rbc { display: ruby-base-container; }
rb { display: ruby-base; white-space: nowrap; }
rt { display: ruby-text; }
rtc { display: ruby-text-container; }
rp { display: none; }
ruby, rb, rt, rbc, rtc { unicode-bidi: isolate; }
rtc, rt { font-variant-east-asian: ruby; white-space: nowrap; line-height: 1; }
rtc, :not(rtc) > rt { font-size: 50%; }
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_stylesheet;

    #[test]
    fn stylesheet_attr_casts_use_each_elements_attributes() {
        let mut dom = Dom::new();
        let html = dom.create_element(NodeIdx(0), "html");
        let body = dom.create_element(html, "body");
        let a = dom.create_element(body, "div");
        let b = dom.create_element(body, "div");
        dom.set_attrs(a, vec![("data-width".into(), "200px".into())]);
        dom.set_attrs(b, vec![("data-width".into(), "bad".into())]);
        let sheet = parse_stylesheet("div { width: attr(data-width type(<length>), 40px); min-width: max(attr(data-width type(<length>))) }");
        let styles = compute(&dom, &[sheet]);
        assert_eq!(styles.get(a).and_then(|style| style.get("width")), Some("200px"));
        assert_eq!(styles.get(b).and_then(|style| style.get("width")), Some("40px"));
        assert_eq!(styles.get(a).and_then(|style| style.get("min-width")), Some("max(200px)"));
    }

    #[test]
    fn root_font_size_math_uses_initial_em_and_rem_basis() {
        let mut dom = Dom::new();
        let html = dom.create_element(NodeIdx(0), "html");
        let body = dom.create_element(html, "body");
        let child = dom.create_element(body, "div");
        let styles = compute(&dom, &[parse_stylesheet(
            "html { font-size: calc(1rem + 1em) } div { font-size: min(2em, 3rem) }",
        )]);
        assert_eq!(styles.get(html).and_then(|style| style.get("font-size")), Some("32px"));
        assert_eq!(styles.get(child).and_then(|style| style.get("font-size")), Some("64px"));
    }

    #[test]
    fn viewport_environment_resolves_lengths_before_root_rem_inheritance() {
        let mut dom = Dom::new();
        let html = dom.create_element(NodeIdx(0), "html");
        let body = dom.create_element(html, "body");
        let child = dom.create_element(body, "div");
        let environment = MediaEnvironment { viewport_width_px: 800.0, viewport_height_px: 600.0,
            small_viewport_px: Some((700.0, 500.0)), ..Default::default() };
        let styles = compute_with_environment(&dom, &[parse_stylesheet(
            "html { font-size: 100vw } div { width: 50svw; height: calc(10vh + 5px); font-size: 1rem }",
        )], &environment);
        assert_eq!(styles.get(html).and_then(|style| style.get("font-size")), Some("800px"));
        assert_eq!(styles.get(child).and_then(|style| style.get("width")), Some("350px"));
        assert_eq!(styles.get(child).and_then(|style| style.get("height")), Some("calc(60px + 5px)"));
        assert_eq!(styles.get(child).and_then(|style| style.get("font-size")), Some("800px"));
    }

    #[test]
    fn viewport_invalidation_selects_only_dependent_nodes() {
        let mut dom = Dom::new();
        let html = dom.create_element(NodeIdx(0), "html");
        let body = dom.create_element(html, "body");
        let dependent = dom.create_element(body, "div");
        dom.set_attrs(dependent, vec![("class".into(), "responsive".into())]);
        let unrelated = dom.create_element(body, "div");
        let sheet = parse_stylesheet(".responsive { width: 50vw } div { color: red }");
        let old = MediaEnvironment { viewport_width_px: 800.0, viewport_height_px: 600.0, ..Default::default() };
        let new = MediaEnvironment { viewport_width_px: 1000.0, ..old };
        let invalidator = StyleInvalidator::new_with_environment(&[sheet.clone()], 7, old);
        let previous = compute_with_environment(&dom, &[sheet.clone()], &old);
        let roots = invalidator.collect_viewport_resize(&dom, new, 7);
        assert_eq!(roots.roots, vec![dependent]);
        let delta = recompute_roots(&dom, &previous, roots);
        assert_eq!(delta.styles.get(dependent).and_then(|style| style.get("width")), Some("500px"));
        assert_eq!(delta.styles.get(unrelated).and_then(|style| style.get("color")), Some("red"));
        assert_eq!(delta.styles.styles, compute_with_environment(&dom, &[sheet], &new).styles);
    }

    #[test]
    fn viewport_resize_recomputes_root_rem_descendants() {
        let mut dom = Dom::new();
        let html = dom.create_element(NodeIdx(0), "html");
        let body = dom.create_element(html, "body");
        let child = dom.create_element(body, "div");
        let sheet = parse_stylesheet("html { font-size: 25vw } div { font-size: 1rem }");
        let old = MediaEnvironment { viewport_width_px: 800.0, ..Default::default() };
        let new = MediaEnvironment { viewport_width_px: 1000.0, ..old };
        let invalidator = StyleInvalidator::new_with_environment(&[sheet.clone()], 9, old);
        let previous = compute_with_environment(&dom, &[sheet.clone()], &old);
        let roots = invalidator.collect_viewport_resize(&dom, new, 9);
        assert_eq!(roots.roots, vec![html]);
        let delta = recompute_roots(&dom, &previous, roots);
        assert_eq!(delta.styles.get(html).and_then(|style| style.get("font-size")), Some("250px"));
        assert_eq!(delta.styles.get(child).and_then(|style| style.get("font-size")), Some("250px"));
        assert_eq!(delta.styles.styles, compute_with_environment(&dom, &[sheet], &new).styles);
    }

    #[test]
    fn media_groups_use_the_computation_environment_and_resize_invalidates() {
        let mut dom = Dom::new();
        let html = dom.create_element(NodeIdx(0), "html");
        let body = dom.create_element(html, "body");
        let target = dom.create_element(body, "div");
        let sheet = parse_stylesheet("div { color: red } @media screen and (min-width: 900px) { @supports (display: grid) { div { color: green } } } @media print { div { color: blue } }");
        let small = MediaEnvironment { viewport_width_px: 800.0, ..Default::default() };
        let large = MediaEnvironment { viewport_width_px: 1000.0, ..small };
        assert_eq!(compute_with_environment(&dom, &[sheet.clone()], &small).get(target).and_then(|s| s.get("color")), Some("red"));
        assert_eq!(compute_with_environment(&dom, &[sheet.clone()], &large).get(target).and_then(|s| s.get("color")), Some("green"));
        let previous = compute_with_environment(&dom, &[sheet.clone()], &small);
        let roots = StyleInvalidator::new_with_environment(&[sheet.clone()], 10, small)
            .collect_viewport_resize(&dom, large, 10);
        assert!(roots.full);
        assert_eq!(recompute_roots(&dom, &previous, roots).styles.styles,
            compute_with_environment(&dom, &[sheet], &large).styles);
    }

    /// html > (head > title, body > h1.title, p#intro, a[href]).
    /// Returns the arena plus [html, head, title, body, h1, p, a].
    fn fixture() -> (Dom, Vec<NodeIdx>) {
        let mut dom = Dom::new();
        let doc = NodeIdx(0);
        let html = dom.create_element(doc, "html");
        let head = dom.create_element(html, "head");
        let title = dom.create_element(head, "title");
        dom.create_text(title, "t");
        let body = dom.create_element(html, "body");
        let h1 = dom.create_element(body, "h1");
        dom.set_attrs(h1, vec![("class".into(), "title".into())]);
        dom.create_text(h1, "Hello");
        let p = dom.create_element(body, "p");
        dom.set_attrs(p, vec![("id".into(), "intro".into())]);
        dom.create_text(p, "World");
        let a = dom.create_element(body, "a");
        dom.set_attrs(a, vec![("href".into(), "/next".into())]);
        (dom, vec![html, head, title, body, h1, p, a])
    }

    #[test]
    fn ua_important_reverses_origin_order_and_ua_revert_defaults() {
        let (dom, elements) = fixture();
        let author = compile_sheets(
            &[parse_stylesheet("p{color:red!important;display:flex}")],
            false,
            &MediaEnvironment::default(),
        );
        for (ua_css, expected) in [
            ("p{color:green!important}", "green"),
            ("p{color:revert!important}", "black"),
            ("p{color:revert-layer!important}", "black"),
            (
                "@layer a,b;@layer a{p{color:revert-layer!important}}@layer b{p{color:green!important}}",
                "black",
            ),
        ] {
            let ua = compile_sheets(&[parse_stylesheet(ua_css)], true, &MediaEnvironment::default());
            let (style, _) = compute_one(&dom, elements[5], None, 16.0, &ua, &author, None, None, None, &MediaEnvironment::default());
            assert_eq!(style.get("color"), Some(expected));
        }
    }

    #[test]
    fn pseudo_styles_content_and_counter_flow() {
        let mut dom = Dom::new();
        let root = dom.create_element(NodeIdx(0), "html");
        let body = dom.create_element(root, "body");
        let ol = dom.create_element(body, "ol");
        let first = dom.create_element(ol, "li");
        dom.set_attrs(first, vec![("data-label".into(), "Alpha".into())]);
        let second = dom.create_element(ol, "li");
        let css = parse_stylesheet(
            r#"
            body { color: blue; quotes: "[" "]"; }
            li { counter-increment: chapter; }
            li::before { content: open-quote attr(data-label) " " counter(chapter) "/" counter(list-item) close-quote; color: red; }
            li::after { content: url("icon.svg") / "icon"; }
            li::marker { color: green; }
        "#,
        );
        let styles = compute(&dom, &[css]);
        assert_eq!(styles.get(first).unwrap().get("color"), Some("blue"));
        assert_eq!(
            styles
                .get_pseudo(first, PseudoElement::Before)
                .unwrap()
                .get("color"),
            Some("red")
        );
        assert_eq!(
            styles
                .generated_content(first, PseudoElement::Before)
                .unwrap()
                .text(),
            "[Alpha 1/1]"
        );
        assert_eq!(
            styles
                .generated_content(second, PseudoElement::Before)
                .unwrap()
                .text(),
            "[ 2/2]"
        );
        assert_eq!(
            styles
                .generated_content(first, PseudoElement::Marker)
                .unwrap()
                .text(),
            "1. "
        );
        let after = styles
            .generated_content(first, PseudoElement::After)
            .unwrap();
        assert_eq!(
            after.runs,
            vec![crate::generated::ContentRun::Image("icon.svg".into())]
        );
        assert_eq!(after.alt_text.as_deref(), Some("icon"));
    }

    #[test]
    fn marker_generation_respects_list_style_type_and_content_none() {
        let mut dom = Dom::new();
        let root = dom.create_element(NodeIdx(0), "html");
        let body = dom.create_element(root, "body");
        let ul = dom.create_element(body, "ul");
        let bullet = dom.create_element(ul, "li");
        let suppressed = dom.create_element(ul, "li");
        dom.set_attrs(suppressed, vec![("class".into(), "suppressed".into())]);
        let css = parse_stylesheet("li.suppressed { list-style-type: none }");
        let styled = compute(&dom, &[css]);
        assert_eq!(styled.generated_content(bullet, PseudoElement::Marker).unwrap().text(), "• ");
        assert!(styled.get_pseudo(suppressed, PseudoElement::Marker).is_none());
    }

    #[test]
    fn counter_style_descriptors_apply_to_markers_and_content() {
        let mut dom = Dom::new();
        let root = dom.create_element(NodeIdx(0), "html");
        let body = dom.create_element(root, "body");
        let ol = dom.create_element(body, "ol");
        let item = dom.create_element(ol, "li");
        let css = parse_stylesheet(
            "@counter-style custom { system: numeric; symbols: 'A' 'B'; prefix: '['; suffix: ']'; } \
             li { list-style-type: custom } li::before { content: counter(list-item, custom) }",
        );
        let styles = compute(&dom, &[css]);
        assert_eq!(styles.generated_content(item, PseudoElement::Marker).unwrap().text(), "[B]");
        assert_eq!(styles.generated_content(item, PseudoElement::Before).unwrap().text(), "B");
    }

    #[test]
    fn quoted_list_style_type_is_literal_marker_text_including_escapes() {
        let mut dom = Dom::new();
        let root = dom.create_element(NodeIdx(0), "html");
        let body = dom.create_element(root, "body");
        let list = dom.create_element(body, "ul");
        let plain = dom.create_element(list, "li");
        let escaped = dom.create_element(list, "li");
        dom.set_attrs(escaped, vec![("class".into(), "escaped".into())]);
        let css = parse_stylesheet(r##"li { list-style-type: "# " } li.escaped { list-style-type: "\23\20" }"##);
        let styled = compute(&dom, &[css]);
        assert_eq!(styled.generated_content(plain, PseudoElement::Marker).unwrap().text(), "# ");
        assert_eq!(styled.generated_content(escaped, PseudoElement::Marker).unwrap().text(), "# ");
    }

    #[test]
    fn generated_images_use_winning_stylesheet_url_and_inherited_marker_image() {
        use crate::generated::ContentRun;
        let mut dom = Dom::new();
        let root = dom.create_element(NodeIdx(0), "html");
        let body = dom.create_element(root, "body");
        let list = dom.create_element(body, "ul");
        let item = dom.create_element(list, "li");
        let mut first = parse_stylesheet("li::before { content: url(icons/old.svg) } ul { list-style-image: url(../marks/bullet.svg) }");
        first.base_url = Some("https://example.test/css/theme/site.css".into());
        let mut second = parse_stylesheet("li::before { content: url(../new.svg) }");
        second.base_url = Some("https://example.test/overrides/over.css".into());
        let styled = compute(&dom, &[first, second]);
        assert_eq!(styled.generated_content(item, PseudoElement::Before).unwrap().runs,
            vec![ContentRun::Image("https://example.test/new.svg".into())]);
        assert_eq!(styled.generated_content(item, PseudoElement::Marker).unwrap().runs,
            vec![ContentRun::Image("https://example.test/css/marks/bullet.svg".into())]);
    }

    #[test]
    fn pseudo_counter_operations_and_hidden_subtrees() {
        let mut dom = Dom::new();
        let root = dom.create_element(NodeIdx(0), "html");
        let body = dom.create_element(root, "body");
        let first = dom.create_element(body, "p");
        dom.set_attrs(first, vec![("class".into(), "first".into())]);
        let hidden = dom.create_element(body, "p");
        dom.set_attrs(hidden, vec![("class".into(), "hidden".into())]);
        let last = dom.create_element(body, "p");
        let css = parse_stylesheet(
            "body { counter-reset: n } p.hidden { display: none; counter-increment: n } p::before { counter-increment: n; content: counter(n) } p.first::after { content: counter(n) }",
        );
        let styles = compute(&dom, &[css]);
        assert_eq!(
            styles
                .generated_content(first, PseudoElement::Before)
                .unwrap()
                .text(),
            "1"
        );
        assert_eq!(
            styles
                .generated_content(first, PseudoElement::After)
                .unwrap()
                .text(),
            "1"
        );
        assert!(
            styles
                .generated_content(hidden, PseudoElement::Before)
                .is_none()
        );
        assert_eq!(
            styles
                .generated_content(last, PseudoElement::Before)
                .unwrap()
                .text(),
            "2"
        );
    }

    #[test]
    fn nested_and_reversed_list_counters_follow_document_order() {
        let mut dom = Dom::new();
        let root = dom.create_element(NodeIdx(0), "html");
        let body = dom.create_element(root, "body");
        let outer = dom.create_element(body, "ol");
        let one = dom.create_element(outer, "li");
        let inner = dom.create_element(one, "ol");
        let nested = dom.create_element(inner, "li");
        let two = dom.create_element(outer, "li");
        let reverse = dom.create_element(body, "ol");
        dom.set_attrs(reverse, vec![("reversed".into(), "".into())]);
        let r1 = dom.create_element(reverse, "li");
        let r2 = dom.create_element(reverse, "li");
        let css = parse_stylesheet(
            "li::before { content: counters(list-item, '.') } li::after { content: counter(list-item, upper-roman) }",
        );
        let styles = compute(&dom, &[css]);
        for (node, expected) in [
            (one, "1"),
            (nested, "1.1"),
            (two, "2"),
            (r1, "2"),
            (r2, "1"),
        ] {
            assert_eq!(
                styles
                    .generated_content(node, PseudoElement::Before)
                    .unwrap()
                    .text(),
                expected
            );
        }
        assert_eq!(
            styles
                .generated_content(two, PseudoElement::After)
                .unwrap()
                .text(),
            "II"
        );
    }

    #[test]
    fn counters_reinstantiate_after_display_contents_reset_scope_ends() {
        let mut dom = Dom::new();
        let root = dom.create_element(NodeIdx(0), "html");
        let body = dom.create_element(root, "body");
        let contents = dom.create_element(body, "div");
        dom.set_attrs(contents, vec![("class".into(), "contents".into())]);
        let reset = dom.create_element(contents, "div");
        dom.set_attrs(reset, vec![("class".into(), "reset".into())]);
        let touch = dom.create_element(contents, "div");
        dom.set_attrs(touch, vec![("class".into(), "touch".into())]);
        let value = dom.create_element(touch, "span");
        let following = dom.create_element(contents, "span");
        let css = parse_stylesheet(".contents { display: contents } .reset { counter-reset: x 4 y 9 } .touch { counter-increment: x 2; counter-set: y 7 } span::before { content: counter(x) '/' counters(y, '.') }");
        let styles = compute(&dom, &[css]);
        for node in [value, following] {
            assert_eq!(styles.generated_content(node, PseudoElement::Before).unwrap().text(), "2/7");
        }
    }

    #[test]
    fn reversed_list_popped_scope_allows_counter_reads_and_later_increment() {
        let mut dom = Dom::new();
        let root = dom.create_element(NodeIdx(0), "html");
        let body = dom.create_element(root, "body");
        let list = dom.create_element(body, "ol");
        dom.set_attrs(list, vec![("reversed".into(), "".into())]);
        let in_list = dom.create_element(list, "li");
        let reader = dom.create_element(body, "p");
        let later = dom.create_element(body, "li");
        let css = parse_stylesheet("p::before { content: counter(list-item) '/' counters(list-item, '.') } li::before { content: counter(list-item) }");
        let styles = compute(&dom, &[css]);
        assert_eq!(styles.generated_content(in_list, PseudoElement::Before).unwrap().text(), "1");
        assert_eq!(styles.generated_content(reader, PseudoElement::Before).unwrap().text(), "0/0");
        assert_eq!(styles.generated_content(later, PseudoElement::Before).unwrap().text(), "1");
    }

    #[test]
    fn reversed_counters_derive_initial_value_and_respect_explicit_start() {
        let mut dom = Dom::new();
        let root = dom.create_element(NodeIdx(0), "html");
        let body = dom.create_element(root, "body");
        let dynamic = dom.create_element(body, "div");
        dom.set_attrs(dynamic, vec![("class".into(), "dynamic".into())]);
        let first = dom.create_element(dynamic, "span");
        let second = dom.create_element(dynamic, "span");
        let explicit = dom.create_element(body, "div");
        dom.set_attrs(explicit, vec![("class".into(), "explicit".into())]);
        let third = dom.create_element(explicit, "span");
        let fourth = dom.create_element(explicit, "span");
        let css = parse_stylesheet(".dynamic { counter-reset: reversed(c) } .explicit { counter-reset: reversed(c) 10 } span { counter-increment: c -1 } span::before { content: counter(c) }");
        let styles = compute(&dom, &[css]);
        for (node, expected) in [(first, "2"), (second, "1"), (third, "9"), (fourth, "8")] {
            assert_eq!(styles.generated_content(node, PseudoElement::Before).unwrap().text(), expected);
        }
    }

    #[test]
    fn generated_content_attribute_fallback_and_unquoted_url() {
        let (dom, elements) = fixture();
        let p = elements[5];
        let css = parse_stylesheet(
            "p::before { content: attr(missing, 'fallback') } p::after { content: url(icon.svg) / attr(id) }",
        );
        let styles = compute(&dom, &[css]);
        assert_eq!(
            styles
                .generated_content(p, PseudoElement::Before)
                .unwrap()
                .text(),
            "fallback"
        );
        let after = styles.generated_content(p, PseudoElement::After).unwrap();
        assert_eq!(
            after.runs,
            vec![crate::generated::ContentRun::Image("icon.svg".into())]
        );
        assert_eq!(after.alt_text.as_deref(), Some("intro"));
    }

    #[test]
    fn generated_content_data_url_and_gradient_image_runs() {
        let (dom, elements) = fixture();
        let p = elements[5];
        let css = parse_stylesheet(
            "p::before { content: url(data:image/png;base64,AA+10/20==) } \
             p::after { content: linear-gradient(red, blue) }",
        );
        let styles = compute(&dom, &[css]);
        assert_eq!(
            styles.generated_content(p, PseudoElement::Before).unwrap().runs,
            vec![crate::generated::ContentRun::Image("data:image/png;base64,AA+10/20==".into())]
        );
        assert_eq!(
            styles.generated_content(p, PseudoElement::After).unwrap().runs,
            vec![crate::generated::ContentRun::Image("linear-gradient(red, blue)".into())]
        );
    }

    #[test]
    fn pseudo_box_generation_and_selector_specificity() {
        let (dom, elements) = fixture();
        let p = elements[5];
        let css = parse_stylesheet(
            "p::before { content: none } p:before { color: red } p::after { content: 'x' } p::first-letter { color: green }",
        );
        let styles = compute(&dom, &[css]);
        assert!(styles.get_pseudo(p, PseudoElement::Before).is_none());
        assert_eq!(
            styles
                .generated_content(p, PseudoElement::After)
                .unwrap()
                .text(),
            "x"
        );
        assert_eq!(
            styles
                .get_pseudo(p, PseudoElement::FirstLetter)
                .unwrap()
                .get("color"),
            Some("green")
        );
        assert_eq!(styles.get(p).unwrap().get("color"), Some("black"));
    }

    #[test]
    fn first_line_and_letter_only_exist_on_matching_originators() {
        let mut dom = Dom::new();
        let root = dom.create_element(NodeIdx(0), "html");
        let body = dom.create_element(root, "body");
        let first = dom.create_element(body, "p");
        dom.set_attrs(first, vec![("class".into(), "chosen".into())]);
        let child = dom.create_element(first, "span");
        let other = dom.create_element(body, "p");
        let css = parse_stylesheet("p.chosen::first-line { color: red } p.chosen::first-letter { color: blue } p.chosen::before { content: none }");
        let styled = compute(&dom, &[css]);
        for pseudo in [PseudoElement::FirstLine, PseudoElement::FirstLetter] {
            assert!(styled.pseudo_matched(first, pseudo));
            assert!(styled.get_pseudo(first, pseudo).is_some());
            assert!(!styled.pseudo_matched(child, pseudo));
            assert!(styled.get_pseudo(child, pseudo).is_none());
            assert!(!styled.pseudo_matched(other, pseudo));
            assert!(styled.get_pseudo(other, pseudo).is_none());
        }
        assert!(styled.pseudo_matched(first, PseudoElement::Before));
        assert!(styled.get_pseudo(first, PseudoElement::Before).is_none());
    }

    #[test]
    fn ua_defaults_apply_without_author_css() {
        let (dom, els) = fixture();
        let styled = compute(&dom, &[]);
        let body = styled.get(els[3]).unwrap();
        assert_eq!(body.get("display"), Some("block"));
        // The UA `margin: 8px` shorthand expands into per-side longhands.
        for side in ["top", "right", "bottom", "left"] {
            assert_eq!(body.get(&format!("margin-{side}")), Some("8px"));
        }
        // Inherited from html's UA rule.
        assert_eq!(body.get("font-family"), Some("serif"));
        assert_eq!(body.get("font-size"), Some("16px"));
        // Non-inherited default, not overridden by UA sheet on body.
        assert_eq!(body.get("background-color"), Some("transparent"));
        // Head is hidden; its children inherit visibility but stay unrendered
        // via display:none at layout time — here just check the rule hit.
        assert_eq!(styled.get(els[1]).unwrap().get("display"), Some("none"));
        // Heading picked up its UA font-size.
        assert_eq!(styled.get(els[4]).unwrap().get("font-size"), Some("32px"));
    }

    #[test]
    fn ruby_defaults_ua_mapping_and_inherited_properties() {
        let mut dom = Dom::new();
        let html = dom.create_element(NodeIdx(0), "html");
        let body = dom.create_element(html, "body");
        let ruby = dom.create_element(body, "ruby");
        let rbc = dom.create_element(ruby, "rbc");
        let rb = dom.create_element(rbc, "rb");
        let rtc = dom.create_element(ruby, "rtc");
        let rt = dom.create_element(rtc, "rt");
        let direct_rt = dom.create_element(ruby, "rt");
        let rp = dom.create_element(ruby, "rp");
        let styled = compute(&dom, &[]);
        for (node, display) in [(ruby, "ruby"), (rbc, "ruby-base-container"),
            (rb, "ruby-base"), (rtc, "ruby-text-container"), (rt, "ruby-text"),
            (rp, "none")] {
            assert_eq!(styled.get(node).unwrap().get("display"), Some(display));
        }
        for (property, initial) in [("ruby-align", "space-around"), ("ruby-position", "alternate"),
            ("ruby-overhang", "auto"), ("ruby-merge", "separate")] {
            assert_eq!(styled.get(rt).unwrap().get(property), Some(initial));
        }
        assert_eq!(styled.get(rb).unwrap().get("white-space"), Some("nowrap"));
        assert_eq!(styled.get(rt).unwrap().get("line-height"), Some("1"));
        assert_eq!(styled.get(rt).unwrap().get("font-size"), Some("8px"));
        assert_eq!(styled.get(direct_rt).unwrap().get("font-size"), Some("8px"));

        let css = parse_stylesheet("ruby { ruby-align: center; ruby-position: under alternate; ruby-overhang: none; ruby-merge: merge } rt { ruby-align: inherit }");
        let styled = compute(&dom, &[css]);
        for node in [rbc, rb, rtc, rt] {
            let style = styled.get(node).unwrap();
            assert_eq!(style.get("ruby-align"), Some("center"));
            assert_eq!(style.get("ruby-position"), Some("alternate under"));
            assert_eq!(style.get("ruby-overhang"), Some("spaces"));
            assert_eq!(style.get("ruby-merge"), Some("merge"));
        }
    }

    #[test]
    fn author_border_shorthand_beats_ua_longhands() {
        // Regression: the UA sheet declares border-*-style:none longhands;
        // without pre-cascade expansion they would outrank an author
        // `border:` shorthand in the flat computed map.
        let (mut dom, els) = fixture();
        dom.set_attrs(
            els[5],
            vec![("style".into(), "border: 5px solid black".into())],
        );
        let styled = compute(&dom, &[]);
        let p = styled.get(els[5]).unwrap();
        assert_eq!(p.get("border-top-width"), Some("5px"));
        assert_eq!(p.get("border-top-style"), Some("solid"));
        assert_eq!(p.get("border-left-color"), Some("black"));
    }

    #[test]
    fn author_beats_ua_specificity_and_order() {
        let (dom, els) = fixture();
        let sheet = parse_stylesheet("h1 { font-size: 32px } .title { font-size: 48px }");
        let styled = compute(&dom, &[sheet]);
        // Class (0,1,0) beats type (0,0,1).
        assert_eq!(styled.get(els[4]).unwrap().get("font-size"), Some("48px"));

        // Equal specificity: later rule wins.
        let sheet = parse_stylesheet(".title { color: red } .title { color: blue }");
        let styled = compute(&dom, &[sheet]);
        assert_eq!(styled.get(els[4]).unwrap().get("color"), Some("blue"));

        // Lower specificity later still loses.
        let sheet = parse_stylesheet(".title { color: red } h1 { color: green }");
        let styled = compute(&dom, &[sheet]);
        assert_eq!(styled.get(els[4]).unwrap().get("color"), Some("red"));
    }

    #[test]
    fn important_overrides_higher_specificity() {
        let (dom, els) = fixture();
        let sheet = parse_stylesheet("#intro { color: green !important } p { color: red }");
        let styled = compute(&dom, &[sheet]);
        assert_eq!(styled.get(els[5]).unwrap().get("color"), Some("green"));
        // And an author !important beats a UA declaration for the same prop:
        let sheet = parse_stylesheet("p { text-decoration: underline wavy !important }");
        // UA gives links underlined text-decoration only for a:link; p has
        // initial none — author important wins over nothing to do here, so
        // just confirm it landed.
        let styled = compute(&dom, &[sheet]);
        assert_eq!(
            styled.get(els[5]).unwrap().get("text-decoration"),
            Some("underline wavy")
        );
    }

    #[test]
    fn inline_style_beats_sheet_unless_important() {
        let mut dom = Dom::new();
        let doc = NodeIdx(0);
        let html = dom.create_element(doc, "html");
        let body = dom.create_element(html, "body");
        let p = dom.create_element(body, "p");
        dom.set_attrs(
            p,
            vec![
                ("id".into(), "x".into()),
                ("style".into(), "color: purple".into()),
            ],
        );
        let sheet = parse_stylesheet("#x { color: gold } p { background-color: silver }");
        let styled = compute(&dom, &[sheet]);
        let st = styled.get(p).unwrap();
        assert_eq!(st.get("color"), Some("purple")); // inline beats #id
        assert_eq!(st.get("background-color"), Some("silver"));

        // !important in a sheet beats non-important inline…
        let sheet = parse_stylesheet("#x { color: olive !important }");
        let styled = compute(&dom, &[sheet]);
        assert_eq!(styled.get(p).unwrap().get("color"), Some("olive"));
        // …but inline !important beats it back.
        dom.set_attrs(
            p,
            vec![
                ("id".into(), "x".into()),
                ("style".into(), "color: purple !important".into()),
            ],
        );
        let styled = compute(&dom, &[parse_stylesheet("#x { color: olive !important }")]);
        assert_eq!(styled.get(p).unwrap().get("color"), Some("purple"));
    }

    #[test]
    fn inheritance_flows_down_the_tree() {
        let mut dom = Dom::new();
        let doc = NodeIdx(0);
        let html = dom.create_element(doc, "html");
        let body = dom.create_element(html, "body");
        let div = dom.create_element(body, "div");
        dom.set_attrs(div, vec![("style".into(), "color: teal".into())]);
        let inner = dom.create_element(div, "span");
        let deep = dom.create_element(inner, "b");
        let styled = compute(&dom, &[]);
        assert_eq!(styled.get(div).unwrap().get("color"), Some("teal"));
        assert_eq!(styled.get(inner).unwrap().get("color"), Some("teal"));
        assert_eq!(styled.get(deep).unwrap().get("color"), Some("teal"));
        // Non-inherited props do NOT flow: span stays display:inline.
        assert_eq!(styled.get(inner).unwrap().get("display"), Some("inline"));
    }

    #[test]
    fn dir_hint_inherits_but_author_direction_overrides_it() {
        let mut dom = Dom::new();
        let div = dom.create_element(NodeIdx(0), "div");
        dom.set_attrs(div, vec![("dir".into(), "rtl".into())]);
        let span = dom.create_element(div, "span");
        dom.set_attrs(span, vec![("style".into(), "direction:ltr".into())]);
        dom.create_text(span, "x");
        let em = dom.create_element(div, "em");
        dom.create_text(em, "y");
        let styled = compute(&dom, &[]);

        assert_eq!(styled.get(div).unwrap().get("direction"), Some("rtl"));
        assert_eq!(styled.get(span).unwrap().get("direction"), Some("ltr"));
        assert_eq!(styled.get(em).unwrap().get("direction"), Some("rtl"));
    }

    #[test]
    fn vertical_text_policy_inherits_to_descendant_text_boxes() {
        let mut dom = Dom::new();
        let root = dom.create_element(NodeIdx(0), "div");
        dom.set_attrs(
            root,
            vec![(
                "style".into(),
                "writing-mode:vertical-rl;text-orientation:upright;text-combine-upright:all".into(),
            )],
        );
        let child = dom.create_element(root, "span");

        let styled = compute(&dom, &[]);
        let child_style = styled.get(child).unwrap();

        assert_eq!(child_style.get("writing-mode"), Some("vertical-rl"));
        assert_eq!(child_style.get("text-orientation"), Some("upright"));
        assert_eq!(child_style.get("text-combine-upright"), Some("all"));
    }

    #[test]
    fn rem_uses_the_computed_root_font_size() {
        let mut dom = Dom::new();
        let html = dom.create_element(NodeIdx(0), "html");
        dom.set_attrs(html, vec![("style".into(), "font-size:20px".into())]);
        let body = dom.create_element(html, "body");
        let paragraph = dom.create_element(body, "p");
        dom.set_attrs(paragraph, vec![("style".into(), "font-size:2rem".into())]);
        let styled = compute(&dom, &[]);

        assert_eq!(
            styled.get(paragraph).unwrap().get("font-size"),
            Some("40px")
        );
    }

    #[test]
    fn invalid_selector_drops_whole_rule() {
        let (dom, els) = fixture();
        // ::before unsupported → second selector invalidates the rule.
        let sheet = parse_stylesheet("h1 { font-weight: normal } h1::before { font-size: 9px }");
        let styled = compute(&dom, &[sheet]);
        // First rule still applies (it was a separate qualified rule)…
        assert_eq!(
            styled.get(els[2]).unwrap().get("font-weight"),
            Some("normal")
        );
        // …and there was never a 9px anywhere.
        assert_ne!(styled.get(els[2]).unwrap().get("font-size"), Some("9px"));

        // One rule, two selectors, one broken: entire rule gone.
        let sheet = parse_stylesheet("h1, :totally-bogus { font-size: 9px }");
        let styled = compute(&dom, &[sheet]);
        assert_ne!(styled.get(els[2]).unwrap().get("font-size"), Some("9px"));
    }

    #[test]
    fn pseudo_class_selectors_match_in_cascade() {
        let (dom, els) = fixture();
        let sheet = parse_stylesheet(
            "body > p:nth-child(2) { letter-spacing: 1px } \
             a:link { text-decoration: underline wavy }",
        );
        let styled = compute(&dom, &[sheet]);
        // p is body's second element child (the h1 precedes it).
        assert_eq!(
            styled.get(els[5]).unwrap().get("letter-spacing"),
            Some("1px")
        );
        // a:link matched.
        assert_eq!(
            styled.get(els[6]).unwrap().get("text-decoration"),
            Some("underline wavy")
        );
    }

    #[test]
    fn delta_view_reports_only_changes() {
        let (mut dom, els) = fixture(); // [html, head, title, body, h1, p, a]
        dom.set_attrs(
            els[5],
            vec![
                ("id".into(), "intro".into()),
                ("style".into(), "color: red".into()),
            ],
        );
        let styled = compute(&dom, &[]);

        let html_delta = styled.delta_from_parent(els[0], None);
        assert!(!html_delta.is_empty()); // root reports everything

        let body_delta = styled.delta_from_parent(els[3], Some(els[0]));
        // body differs from html by display/margin; font props are equal.
        assert!(
            body_delta
                .iter()
                .any(|(k, v)| k == "margin-top" && v == "8px")
        );
        assert!(body_delta.iter().all(|(k, _)| k != "font-family"));

        // p vs body: color override + UA margin difference. Both are
        // display:block per the UA sheet, so display is not a delta.
        let p_delta = styled.delta_from_parent(els[5], Some(els[3]));
        for (prop, want) in [("color", "red"), ("margin-top", "1em")] {
            assert!(
                p_delta.iter().any(|(k, v)| k == prop && v == want),
                "p delta missing {prop}: {want} — got {p_delta:?}"
            );
        }
        // Inherited-equal and display-equal props stay out of the delta.
        assert!(p_delta.iter().all(|(k, _)| k != "font-size"));
        assert!(p_delta.iter().all(|(k, _)| k != "display"));
    }

    #[test]
    fn ua_sheet_parses_with_supported_selectors_only() {
        // Guards against the built-in sheet drifting into syntax the
        // selector engine can't handle (which would silently drop rules).
        let sheet = parse_stylesheet(UA_SHEET);
        assert!(!sheet.rules.is_empty());
        for rule in &sheet.rules {
            for src in &rule.selector_srcs {
                assert!(
                    parse_selector(src).is_some(),
                    "UA sheet selector not supported: {src}"
                );
            }
        }
    }

    #[test]
    fn later_declarations_win_within_one_rule() {
        let (dom, els) = fixture();
        let styled = compute(&dom, &[parse_stylesheet("p { color:red; color:green }")]);
        assert_eq!(styled.get(els[5]).unwrap().get("color"), Some("green"));
    }

    #[test]
    fn selector_list_uses_the_most_specific_selector_that_actually_matches() {
        let (mut dom, els) = fixture();
        dom.set_attrs(
            els[5],
            vec![
                ("id".into(), "intro".into()),
                ("class".into(), "target".into()),
            ],
        );
        let styled = compute(
            &dom,
            &[parse_stylesheet(
                "p, #does-not-match { color:red } .target { color:green }",
            )],
        );
        assert_eq!(styled.get(els[5]).unwrap().get("color"), Some("green"));
    }

    #[test]
    fn custom_properties_preserve_case_inherit_and_substitute() {
        let (dom, els) = fixture();
        let styled = compute(
            &dom,
            &[parse_stylesheet(
                ":root { --A:green; --a:crimson } p { color:var(--A); background-color:var(--a) }",
            )],
        );
        let paragraph = styled.get(els[5]).unwrap();
        assert_eq!(paragraph.get("color"), Some("green"));
        assert_eq!(paragraph.get("background-color"), Some("crimson"));
    }

    #[test]
    fn invalid_var_winner_defaults_instead_of_reviving_previous_declaration() {
        let (dom, els) = fixture();
        let styled = compute(
            &dom,
            &[parse_stylesheet(
                "body { color:blue } p { color:green; color:var(--missing) }",
            )],
        );
        assert_eq!(styled.get(els[5]).unwrap().get("color"), Some("blue"));
    }

    #[test]
    fn all_resets_supported_properties_but_not_direction_or_unicode_bidi() {
        let (dom, els) = fixture();
        let styled = compute(
            &dom,
            &[parse_stylesheet(
                "p { color:red; background-color:blue; margin:10px; direction:rtl; \
                 unicode-bidi:bidi-override; all:initial; color:green }",
            )],
        );
        let paragraph = styled.get(els[5]).unwrap();
        assert_eq!(paragraph.get("color"), Some("green"));
        assert_eq!(paragraph.get("background-color"), Some("transparent"));
        assert_eq!(paragraph.get("margin-left"), Some("0"));
        assert_eq!(paragraph.get("direction"), Some("rtl"));
        assert_eq!(paragraph.get("unicode-bidi"), Some("bidi-override"));
    }

    #[test]
    fn all_does_not_reset_inherited_custom_properties() {
        let (dom, els) = fixture();
        let styled = compute(
            &dom,
            &[parse_stylesheet(
                ":root { --tone:green } p { all:initial; color:var(--tone) }",
            )],
        );
        assert_eq!(styled.get(els[5]).unwrap().get("color"), Some("green"));
    }
}
