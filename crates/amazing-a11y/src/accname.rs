//! Accessible name and description computation (accname-1.2) and the
//! HTML-AAM host-language text alternatives.
//!
//! The algorithm is the recursive `name from content` walk from the spec:
//!
//! 1. hidden nodes are skipped unless directly referenced,
//! 2. `aria-labelledby` then `aria-label`,
//! 3. HTML host-language alternatives (`label`, `alt`, `caption`, `legend`,
//!    `figcaption`, `title`, `placeholder`),
//! 4. name from content for roles that support it, or any directly referenced
//!    node,
//! 5. `title` as the last resort.
//!
//! Cycles are broken with a visited set; the result is whitespace-flattened.

use amazing_css::StyledDom;
use amazing_css::selectors::PseudoElement;
use amazing_dom::{Dom, NodeIdx, NodeKind};
use std::collections::HashMap;

use crate::host;
use crate::role::{self, Role};

/// Shared context for name computation: the arena, the computed styles (for
/// hidden state and CSS generated content), an id → node index, the labels
/// targeting each id (`<label for=…>`), and (A11Y-13) `aria-owns` target →
/// owner, "first owner wins" in document order (mirroring
/// `tree::Builder::apply_owns`'s own precedence, but computed ahead of it —
/// see `collect_content`/`collect_child`'s A11Y-13 notes for why
/// name-from-content needs its own copy of this rather than waiting for
/// `apply_owns`'s AX-tree reparenting to run first).
pub struct NameContext<'a> {
    pub dom: &'a Dom,
    pub styled: &'a StyledDom,
    pub ids: &'a HashMap<String, NodeIdx>,
    pub labels_by_target: &'a HashMap<String, Vec<NodeIdx>>,
    pub owned_by: &'a HashMap<NodeIdx, NodeIdx>,
}

impl<'a> NameContext<'a> {
    pub fn new(
        dom: &'a Dom,
        styled: &'a StyledDom,
        ids: &'a HashMap<String, NodeIdx>,
        labels_by_target: &'a HashMap<String, Vec<NodeIdx>>,
        owned_by: &'a HashMap<NodeIdx, NodeIdx>,
    ) -> Self {
        Self { dom, styled, ids, labels_by_target, owned_by }
    }
}

/// Accessible name of an element, per accname-1.2.
pub fn accessible_name(ctx: &NameContext<'_>, node: NodeIdx) -> String {
    let mut visited = Vec::new();
    compute_name(ctx, node, &mut visited, false)
}

/// Accessible description of an element, per accname-1.2 §4.3.
pub fn accessible_description(ctx: &NameContext<'_>, node: NodeIdx) -> String {
    let name = accessible_name(ctx, node);
    if let Some(ids) = ctx.dom.get_attribute_ns(node, None, "aria-describedby") {
        let mut parts = Vec::new();
        for id in ids.split_ascii_whitespace() {
            if let Some(target) = ctx.ids.get(id) {
                let mut branch = vec![node];
                let text = compute_name(ctx, *target, &mut branch, true);
                if !text.is_empty() {
                    parts.push(text);
                }
            }
        }
        if !parts.is_empty() {
            return flatten(&parts.join(" "));
        }
    }
    if let Some(description) = non_empty(ctx.dom.get_attribute_ns(node, None, "aria-description")) {
        return flatten(&description);
    }
    if let Some(title) = host::tooltip(ctx.dom, node) {
        let title = flatten(&title);
        if !title.is_empty() && title != name {
            return title;
        }
    }
    String::new()
}

fn compute_name(
    ctx: &NameContext<'_>,
    node: NodeIdx,
    visited: &mut Vec<NodeIdx>,
    referenced: bool,
) -> String {
    if visited.contains(&node) {
        return String::new();
    }
    match &ctx.dom.get(node).kind {
        NodeKind::Text { data } | NodeKind::CDATASection { data } => return flatten(data),
        NodeKind::Element { .. } => {}
        _ => return String::new(),
    }
    // Step 2A: hidden nodes produce no name unless directly referenced.
    if !referenced && is_hidden(ctx, node) {
        return String::new();
    }
    visited.push(node);

    // Step 2B: aria-labelledby (referenced nodes may be hidden). A `self`
    // reference in the id list cannot recurse into 2B again (infinite loop),
    // so it is resolved via `self_text_alternative` (steps 2D-2G only)
    // instead of being dropped outright.
    if let Some(ids) = ctx.dom.get_attribute_ns(node, None, "aria-labelledby") {
        let mut parts = Vec::new();
        for id in ids.split_ascii_whitespace() {
            if let Some(target) = ctx.ids.get(id) {
                let text = if *target == node {
                    self_text_alternative(ctx, node)
                } else {
                    let mut branch = visited.clone();
                    compute_name(ctx, *target, &mut branch, true)
                };
                if !text.is_empty() {
                    parts.push(text);
                }
            }
        }
        if !parts.is_empty() {
            visited.pop();
            // Distinct id references are separate labels and are joined with
            // a space (unlike step 2F's natural-flow concatenation below):
            // each is already a self-contained, individually flattened
            // fragment with no boundary whitespace of its own to rely on.
            return flatten(&parts.join(" "));
        }
    }

    let element_role = role::role_for(ctx.dom, ctx.ids, node);

    // Step 2D: aria-label.
    if let Some(label) = non_empty(ctx.dom.get_attribute_ns(node, None, "aria-label")) {
        visited.pop();
        return flatten(&label);
    }

    // Step 2E: HTML host-language text alternative.
    if let Some(alt) = host_language_name(ctx, node) {
        visited.pop();
        return flatten(&alt);
    }

    // Tentative "name from heading": `alertdialog`/`article`/`dialog` (ARIA
    // role or native `<dialog>`) derive their name from the first heading
    // found via DOM-tree-order DFS among descendants, when nothing more
    // specific applies (https://github.com/w3c/aria/pull/1860).
    if matches!(element_role, Role::Alertdialog | Role::Article | Role::Dialog)
        && let Some(heading_name) = name_from_first_heading(ctx, node, visited)
    {
        visited.pop();
        return heading_name;
    }

    // Steps 2F/2G: name from content, for roles that allow it or when the node
    // is directly referenced by aria-labelledby/aria-describedby.
    if referenced || element_role.name_from_content() {
        let mut parts = Vec::new();
        let flags = ContentFlags {
            embed_control_values: ctx.dom.name(node) == Some("label"),
            exclude: None,
            force_include: referenced && is_hidden(ctx, node),
        };
        collect_content(ctx, node, visited, &mut parts, flags);
        if !parts.is_empty() {
            visited.pop();
            return flatten(&parts.concat());
        }
    }

    // Step 2I: title, except for roles that prohibit naming.
    if !matches!(element_role, Role::Generic | Role::None | Role::Presentation)
        && let Some(title) = host::tooltip(ctx.dom, node)
    {
        visited.pop();
        return flatten(&title);
    }

    visited.pop();
    String::new()
}

/// The self-reference case of `aria-labelledby="… self …"`: steps 2D-2G,
/// skipping 2B itself (it is already in progress for this exact node, so
/// recursing back into it would either infinite-loop or, via the visited-set
/// guard, always return empty — neither of which is what implementations or
/// WPT expect from e.g. `<div id="g2" aria-label="self label"
/// aria-labelledby="g2 h2">`, which contributes "self label" for the `g2`
/// token).
fn self_text_alternative(ctx: &NameContext<'_>, node: NodeIdx) -> String {
    if let Some(label) = non_empty(ctx.dom.get_attribute_ns(node, None, "aria-label")) {
        return flatten(&label);
    }
    if let Some(alt) = host_language_name(ctx, node) {
        return flatten(&alt);
    }
    if role::role_for(ctx.dom, ctx.ids, node).name_from_content() {
        let mut visited = vec![node];
        let mut parts = Vec::new();
        let flags = ContentFlags { embed_control_values: false, exclude: None, force_include: false };
        collect_content(ctx, node, &mut visited, &mut parts, flags);
        if !parts.is_empty() {
            return flatten(&parts.concat());
        }
    }
    String::new()
}

/// DFS over `node`'s descendants (document order, `node` excluded) for the
/// first element with role `Heading`, returning its own accessible name.
/// Used by the tentative "name from heading" step for `alertdialog`/
/// `article`/`dialog` (https://github.com/w3c/aria/pull/1860).
fn name_from_first_heading(
    ctx: &NameContext<'_>,
    node: NodeIdx,
    visited: &mut Vec<NodeIdx>,
) -> Option<String> {
    for descendant in ctx.dom.tree_order(node) {
        if descendant == node || role::role_for(ctx.dom, ctx.ids, descendant) != Role::Heading {
            continue;
        }
        let mut branch = visited.clone();
        let name = compute_name(ctx, descendant, &mut branch, false);
        if !name.is_empty() {
            return Some(name);
        }
    }
    None
}

/// HTML-AAM host language alternatives, in precedence order.
fn host_language_name(ctx: &NameContext<'_>, node: NodeIdx) -> Option<String> {
    let dom = ctx.dom;
    let tag = dom.name(node)?;
    match tag {
        "img" | "area" => match dom.get_attribute_ns(node, None, "alt") {
            // An `alt` attribute present (even empty) is the name; otherwise
            // fall back to `title`, then (A11Y-10) to an ancestor `<figure>`'s
            // `<figcaption>` when this element is its sole non-whitespace
            // content besides that caption.
            Some(alt) => Some(alt.to_owned()),
            None => non_empty(dom.get_attribute_ns(node, None, "title"))
                .or_else(|| figure_figcaption_fallback(dom, node)),
        },
        "input" => {
            let ty = dom
                .get_attribute_ns(node, None, "type")
                .unwrap_or("text")
                .to_ascii_lowercase();
            if ty == "image" {
                return non_empty(dom.get_attribute_ns(node, None, "alt"))
                    .or_else(|| non_empty(dom.get_attribute_ns(node, None, "title")));
            }
            if ty == "hidden" {
                return Option::None;
            }
            if matches!(ty.as_str(), "button" | "submit" | "reset") {
                if let Some(value) = non_empty(dom.get_attribute_ns(node, None, "value")) {
                    return Some(value);
                }
                return match ty.as_str() {
                    "submit" => Some("Submit".to_owned()),
                    "reset" => Some("Reset".to_owned()),
                    _ => Option::None,
                };
            }
            if let Some(label) = associated_label_text(ctx, node) {
                return Some(label);
            }
            if let Some(title) = non_empty(dom.get_attribute_ns(node, None, "title")) {
                return Some(title);
            }
            non_empty(dom.get_attribute_ns(node, None, "placeholder"))
        }
        "textarea" => {
            if let Some(label) = associated_label_text(ctx, node) {
                return Some(label);
            }
            if let Some(title) = non_empty(dom.get_attribute_ns(node, None, "title")) {
                return Some(title);
            }
            non_empty(dom.get_attribute_ns(node, None, "placeholder"))
        }
        "select" => {
            if let Some(label) = associated_label_text(ctx, node) {
                return Some(label);
            }
            non_empty(dom.get_attribute_ns(node, None, "title"))
        }
        "table" => child_text_by_tag(dom, node, "caption"),
        "fieldset" => child_text_by_tag(dom, node, "legend"),
        // A11Y-10: unlike `table`/`caption` and `fieldset`/`legend`, a
        // `<figure>`'s OWN name is not derived from its `<figcaption>` at
        // all (only `aria-label`/`aria-labelledby`/`title`, i.e. no
        // host-language mapping here — the figcaption text is instead an
        // `<img>`-only fallback, see `figure_figcaption_fallback` below).
        "svg" => descendant_text_by_tag(dom, node, "title"),
        "abbr" => non_empty(dom.get_attribute_ns(node, None, "title")),
        "progress" | "meter" | "output" => associated_label_text(ctx, node),
        _ => None,
    }
}

/// Per-traversal switches for [`collect_content`]/[`collect_child`], grouped
/// so a single recursive call site doesn't need five positional bools.
#[derive(Clone, Copy)]
struct ContentFlags {
    /// Set for a `<label>`'s own traversal (accname-1.2 §2.5.3): a labelable
    /// descendant's current value is substituted for its own text. Unset for
    /// ordinary name-from-content roles (`<button>`, `<h1>`, …), whose
    /// embedded form controls contribute nothing, matching
    /// `form_control_text_is_not_container_content`.
    embed_control_values: bool,
    /// The specific control this traversal is computing the name *of* (its
    /// wrapping `<label>`), which never substitutes its own value even when
    /// `embed_control_values` is set — that would fold a text control's
    /// current value into its own name, which accname does not do.
    exclude: Option<NodeIdx>,
    /// Set once inside the subtree of an `aria-labelledby`/`aria-describedby`
    /// target that was itself hidden: per accname-1.2 §2B's note, "user
    /// agents MUST include all nodes in the subtree… when the node
    /// referenced… is hidden", so every hidden check is skipped for the rest
    /// of this subtree, however deeply nested. It never turns back off.
    force_include: bool,
}

/// Appends the name-from-content pieces of `node`: its `::marker`/`::before`
/// generated content, text, embedded-control values, and descendants, then
/// `::after`. Pieces are pushed *raw* (not individually flattened/trimmed) so
/// that boundary whitespace between fragments survives to the single
/// `flatten` call the caller makes on the concatenated result — flattening
/// each fragment here would both lose genuine separating whitespace (an
/// author's `content: " before "` padding, or a text node's own trailing
/// space) and — via a forced join separator — invent whitespace that was
/// never in the source.
fn collect_content(
    ctx: &NameContext<'_>,
    node: NodeIdx,
    visited: &mut Vec<NodeIdx>,
    parts: &mut Vec<String>,
    flags: ContentFlags,
) {
    if let Some(marker) = ctx.styled.generated_content(node, PseudoElement::Marker) {
        push_generated(parts, marker);
    }
    if let Some(before) = ctx.styled.generated_content(node, PseudoElement::Before) {
        push_generated(parts, before);
    }
    for child in ctx.dom.children(node) {
        collect_child(ctx, node, child, visited, parts, flags);
    }
    // A11Y-13 (w3c/aria PR #1839, `accname/aria-owns.html`): `aria-owns`
    // targets are appended after `node`'s real DOM children, matching
    // `tree::Builder::apply_owns`'s own tree shape — an owned target is a
    // *relocated* AX child, not a second copy of a real one, so a target
    // that's already a genuine DOM child of `node` (already visited by the
    // loop above) or claimed by a different owner (`ctx.owned_by`, first-
    // owner-wins) is skipped here.
    if let Some(references) = ctx.dom.get_attribute_ns(node, None, "aria-owns") {
        for id in references.split_ascii_whitespace() {
            let Some(&target) = ctx.ids.get(id) else { continue };
            if target == node || ctx.dom.get(target).parent == Some(node) {
                continue;
            }
            if ctx.owned_by.get(&target) != Some(&node) {
                continue;
            }
            collect_child(ctx, node, target, visited, parts, flags);
        }
    }
    if let Some(after) = ctx.styled.generated_content(node, PseudoElement::After) {
        push_generated(parts, after);
    }
}

/// One child of `parent` during name-from-content collection.
///
/// - A text/CDATA node contributes its raw data, transformed per `parent`'s
///   CSS `text-transform`.
/// - Unless `flags.force_include`: `display:none`/`[hidden]`/`aria-hidden`
///   hide the whole subtree unconditionally; `visibility:hidden`/`collapse`
///   excludes this element's own contribution but not a descendant that
///   resets `visibility:visible` — the computed `visibility` on each element
///   already resolves that override, so this just tunnels through (dropping
///   only `child`'s own direct text/generated content, not visible
///   grandchildren), mirroring `tree.rs`'s `Decision::Promote`.
/// - A `display:block`/`inline-block`/… child is a word-boundary: a space is
///   inserted around its contribution even with no source whitespace,
///   matching how block layout visually separates runs.
/// - An embedded form control (other than `flags.exclude`) substitutes its
///   current value when `flags.embed_control_values` is set; otherwise it is
///   skipped, contributing nothing.
/// - Otherwise, an element with its own overriding name (`own_name`) uses
///   that name atomically; anything else recurses.
/// True when `child` is one of `parent`'s own `aria-actions` IDREF targets
/// (A11Y-12). `parent` here is always `child`'s immediate DOM parent at
/// whichever recursion level `collect_child` is currently at — `aria-actions`
/// only excludes a host's *direct* AX children, not arbitrarily nested
/// descendants, so re-checking `parent`'s own attribute at each level (rather
/// than carrying a fixed target set from the root) is exactly the right
/// scope, and cheap: the common case has no `aria-actions` attribute at all.
fn is_aria_action_target(ctx: &NameContext<'_>, parent: NodeIdx, child: NodeIdx) -> bool {
    ctx.dom.get_attribute_ns(parent, None, "aria-actions").is_some_and(|ids| {
        ids.split_ascii_whitespace().any(|id| ctx.ids.get(id) == Some(&child))
    })
}

fn collect_child(
    ctx: &NameContext<'_>,
    parent: NodeIdx,
    child: NodeIdx,
    visited: &mut Vec<NodeIdx>,
    parts: &mut Vec<String>,
    flags: ContentFlags,
) {
    match &ctx.dom.get(child).kind {
        NodeKind::Text { data } | NodeKind::CDATASection { data } => {
            let text = apply_text_transform(ctx, parent, data);
            if !text.is_empty() {
                parts.push(text);
            }
        }
        NodeKind::Element { .. } => {
            // A11Y-12 (aria-actions, w3c/aria#2208 / #1805): a direct AX
            // child that `parent` itself lists in `aria-actions` names its
            // own action, not the host — excluded before any other check
            // (own_name/aria-label included, not just plain recursion; see
            // `aria-actions-target-accname-from-aria-label.tentative.html`,
            // whose target carries an `aria-label` that must still not
            // bleed into the host's name).
            if is_aria_action_target(ctx, parent, child) {
                return;
            }
            // A11Y-13: `child` has been relocated elsewhere by some other
            // element's `aria-owns` (`ctx.owned_by`, first-owner-wins) — it
            // no longer lives here in the AX tree, so it must not
            // contribute to `parent`'s name-from-content from its old DOM
            // position (`accname/aria-owns.html`'s "excludes content
            // relocated by aria-owns" case). When `child` is instead being
            // visited *as* one of `parent`'s own owned targets (the
            // pseudo-child loop `collect_content` added after this file's
            // real DOM children), `ctx.owned_by.get(&child) == Some(parent)`
            // and this check does not fire.
            if ctx.owned_by.get(&child).is_some_and(|&owner| owner != parent) {
                return;
            }
            let already_visited = visited.contains(&child);
            if !flags.force_include && !already_visited && is_hard_hidden(ctx, child) {
                return;
            }
            if !flags.force_include && !already_visited && is_soft_hidden(ctx, child) {
                for grandchild in ctx.dom.children(child) {
                    if matches!(ctx.dom.get(grandchild).kind, NodeKind::Element { .. }) {
                        collect_child(ctx, child, grandchild, visited, parts, flags);
                    }
                    // Text/CDATA directly under a `visibility:hidden` element
                    // has no style of its own to reassert visibility with, so
                    // it stays excluded even though a visible grandchild
                    // element could still override.
                }
                return;
            }

            let boundary = is_block_boundary(ctx, child);
            if boundary {
                parts.push(" ".to_owned());
            }
            // Value substitution is tried before `own_name`: an embedded
            // ARIA widget (`role=combobox`/`slider`/`spinbutton`/`listbox`
            // on a plain `<span>`/`<div>`/`<ul>`) commonly *also* carries its
            // own `aria-label` (as a value description, e.g. "number of
            // times") — that is a different, narrower name than the one this
            // traversal wants (its current value, e.g. "3"), so it must not
            // pre-empt the substitution the way it would for `own_name`'s
            // ordinary atomic-name role.
            // Not gated on `host::is_focusable`: a listbox's `value`
            // substitution applies to the (non-focusable, roving-tabindex)
            // container itself, not to whichever `option` currently holds
            // focus.
            if flags.embed_control_values
                && Some(child) != flags.exclude
                && let Some(value) = control_value(ctx.dom, ctx.ids, child)
            {
                parts.push(value);
            } else if let Some(own) = own_name(ctx, child, visited) {
                if !own.is_empty() {
                    parts.push(own);
                }
            } else if !is_form_control(ctx.dom, child) {
                let mut branch = visited.clone();
                branch.push(child);
                collect_content(ctx, child, &mut branch, parts, flags);
            }
            if boundary {
                parts.push(" ".to_owned());
            }
        }
        _ => {}
    }
}

/// Block-level (or block-like) elements are a word boundary in name-from-
/// content flow even with no whitespace text node between them and their
/// neighbor, matching `display:block`/`inline-block`/`flex`/`grid`/…
/// visually separating runs the way an inline run does not.
fn is_block_boundary(ctx: &NameContext<'_>, node: NodeIdx) -> bool {
    ctx.styled.get(node).and_then(|style| style.get("display")).is_some_and(|display| {
        matches!(
            display.to_ascii_lowercase().as_str(),
            "block"
                | "inline-block"
                | "flex"
                | "inline-flex"
                | "grid"
                | "inline-grid"
                | "list-item"
                | "table"
        )
    })
}

/// CSS `text-transform`, applied to a text node's raw data ahead of the
/// single flattening pass at the end of name computation.
fn apply_text_transform(ctx: &NameContext<'_>, parent: NodeIdx, text: &str) -> String {
    let Some(transform) = ctx.styled.get(parent).and_then(|style| style.get("text-transform"))
    else {
        return text.to_owned();
    };
    if transform.eq_ignore_ascii_case("uppercase") {
        text.to_uppercase()
    } else if transform.eq_ignore_ascii_case("lowercase") {
        text.to_lowercase()
    } else if transform.eq_ignore_ascii_case("capitalize") {
        capitalize_words(text)
    } else {
        text.to_owned()
    }
}

/// Upper-cases the first character of each whitespace-separated word.
fn capitalize_words(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at_word_start = true;
    for ch in text.chars() {
        if ch.is_whitespace() {
            at_word_start = true;
            out.push(ch);
        } else if at_word_start {
            out.extend(ch.to_uppercase());
            at_word_start = false;
        } else {
            out.push(ch);
        }
    }
    out
}

/// The name an element contributes *without* exposing its subtree text: its
/// own `aria-label`/`aria-labelledby`, `<img alt>`, SVG title, and so on.
fn own_name(ctx: &NameContext<'_>, node: NodeIdx, visited: &mut Vec<NodeIdx>) -> Option<String> {
    if visited.contains(&node) {
        return None;
    }
    if ctx.dom.get_attribute_ns(node, None, "aria-labelledby").is_some()
        || non_empty(ctx.dom.get_attribute_ns(node, None, "aria-label")).is_some()
    {
        // `compute_name` pushes `node` onto its own `visited` copy once it
        // gets past the hidden check — pre-pushing it here too would trip
        // its "already visited" guard immediately and always return empty.
        let mut branch = visited.clone();
        return Some(compute_name(ctx, node, &mut branch, false));
    }
    let tag = ctx.dom.name(node)?;
    match tag {
        "img" | "area" | "abbr" => host_language_name(ctx, node),
        "input"
            if ctx
                .dom
                .get_attribute_ns(node, None, "type")
                .is_some_and(|t| t.eq_ignore_ascii_case("image")) =>
        {
            host_language_name(ctx, node)
        }
        _ => None,
    }
}

/// Pushed *raw*: an author's `content: " before "` padding is intentional
/// boundary whitespace, not noise to trim before the final `flatten`.
fn push_generated(parts: &mut Vec<String>, content: &amazing_css::generated::GeneratedContent) {
    let text = match &content.alt_text {
        // The CSS Content alternative-text syntax (`content: X / Y`) hands Y
        // to the accessible name computation as a discrete labelled unit —
        // e.g. a bare `counter()` value with no author-authored padding —
        // rather than literal in-flow content, so unlike a plain string
        // `content` value it gets a trailing word-boundary space rather than
        // relying on the author to have included one.
        Some(alt) if !alt.is_empty() => format!("{alt} "),
        Some(_) => return,
        None => content.text(),
    };
    if !text.is_empty() {
        parts.push(text);
    }
}

/// Controls whose own text content is not part of a container's
/// name-from-content (their "text" is really a value, addressed separately
/// via `control_value`/embedded-control substitution). `<button>` is
/// deliberately excluded: unlike `input`/`select`/`textarea`, its content
/// *is* ordinary name-from-content — nested interactive/structural content
/// (an image, another labelled control) still contributes normally, matching
/// `comp_name_from_content.html`'s "for each child including nested button"
/// cases.
fn is_form_control(dom: &Dom, node: NodeIdx) -> bool {
    matches!(dom.name(node), Some("input" | "select" | "textarea"))
}

fn control_value(dom: &Dom, ids: &HashMap<String, NodeIdx>, node: NodeIdx) -> Option<String> {
    let role = role::role_for(dom, ids, node);
    if matches!(role, Role::Checkbox | Role::Radio | Role::Switch) {
        return None;
    }
    if let Some(value) = non_empty(dom.get_attribute_ns(node, None, "aria-valuetext"))
        .or_else(|| non_empty(dom.get_attribute_ns(node, None, "value")))
        // `aria-valuetext` is a human-readable override of `aria-valuenow`;
        // absent that, the plain numeric `aria-valuenow` is the value
        // (range widgets — slider, spinbutton, progressbar, scrollbar).
        .or_else(|| non_empty(dom.get_attribute_ns(node, None, "aria-valuenow")))
    {
        return Some(value);
    }
    // `<select>`'s value is its selected `<option>`'s text — unlike
    // `<input>`, there is no `value` attribute on the select itself.
    if dom.name(node) == Some("select") {
        return selected_native_option_text(dom, node);
    }
    // An ARIA `listbox`'s value is likewise its selected `option`
    // descendant's text (`aria-selected="true"`), not an attribute of the
    // listbox itself.
    if role == Role::Listbox {
        return selected_listbox_option_text(dom, ids, node);
    }
    // An ARIA `combobox` with no `value`/`aria-valuetext` of its own (e.g. a
    // plain `<span role="combobox">3</span>`) exposes its rendered text as
    // its value.
    if role == Role::Combobox {
        let text = flatten(&host::child_text(dom, node));
        if !text.is_empty() {
            return Some(text);
        }
    }
    None
}

/// The text of a `<select>`'s selected `<option>`, or its first option per
/// the HTML default when none carries `selected`.
fn selected_native_option_text(dom: &Dom, select: NodeIdx) -> Option<String> {
    let options: Vec<NodeIdx> = dom.children(select).filter(|child| dom.name(*child) == Some("option")).collect();
    let selected = options
        .iter()
        .find(|option| dom.get_attribute_ns(**option, None, "selected").is_some())
        .or_else(|| options.first())
        .copied()?;
    let text = flatten(&host::child_text(dom, selected));
    if text.is_empty() { None } else { Some(text) }
}

/// The text of an ARIA `listbox`'s selected `option` descendant.
fn selected_listbox_option_text(dom: &Dom, ids: &HashMap<String, NodeIdx>, listbox: NodeIdx) -> Option<String> {
    for id in dom.tree_order(listbox) {
        if id != listbox
            && role::role_for(dom, ids, id) == Role::Option
            && dom
                .get_attribute_ns(id, None, "aria-selected")
                .is_some_and(|v| v.eq_ignore_ascii_case("true"))
        {
            let text = flatten(&host::child_text(dom, id));
            return if text.is_empty() { None } else { Some(text) };
        }
    }
    None
}

/// Text of all `<label>` elements associated with a labelable control
/// (`for` attribute, or an ancestor `<label>`), concatenated.
fn associated_label_text(ctx: &NameContext<'_>, node: NodeIdx) -> Option<String> {
    if !host::is_labelable(ctx.dom, node) {
        return None;
    }
    let mut labels: Vec<NodeIdx> = Vec::new();
    if let Some(id) = ctx.dom.get_attribute_ns(node, None, "id")
        && let Some(found) = ctx.labels_by_target.get(id)
    {
        labels.extend(found.iter().copied());
    }
    let mut cursor = ctx.dom.get(node).parent;
    while let Some(parent) = cursor {
        if ctx.dom.name(parent) == Some("label") {
            labels.push(parent);
            break;
        }
        cursor = ctx.dom.get(parent).parent;
    }
    labels.sort_unstable();
    labels.dedup();

    // Each `<label>`'s own text is flattened independently (its content is a
    // self-contained fragment); multiple labels for the same control are
    // then joined with a space, per accname-1.2 host-language text
    // alternatives for HTML.
    let mut label_texts = Vec::new();
    for label in labels {
        let mut branch = Vec::new();
        let mut parts = Vec::new();
        let flags = ContentFlags { embed_control_values: true, exclude: Some(node), force_include: false };
        collect_content(ctx, label, &mut branch, &mut parts, flags);
        let text = flatten(&parts.concat());
        if !text.is_empty() {
            label_texts.push(text);
        }
    }
    if label_texts.is_empty() { None } else { Some(label_texts.join(" ")) }
}

fn child_text_by_tag(dom: &Dom, node: NodeIdx, tag: &str) -> Option<String> {
    dom.children(node)
        .find(|child| dom.name(*child) == Some(tag))
        .map(|child| host::child_text(dom, child))
        .filter(|text| !text.trim().is_empty())
}

fn descendant_text_by_tag(dom: &Dom, node: NodeIdx, tag: &str) -> Option<String> {
    for id in dom.tree_order(node) {
        if id != node && dom.name(id) == Some(tag) {
            let text = host::child_text(dom, id);
            if !text.trim().is_empty() {
                return Some(text);
            }
        }
    }
    None
}

/// A11Y-10: an `<img>`/`<area>` with no `alt`/`title` of its own inherits its
/// ancestor `<figure>`'s `<figcaption>` text, but only when it is that
/// figure's sole non-whitespace content besides the figcaption itself — if
/// any other flow content sits alongside it, the figcaption is understood to
/// caption the whole figure rather than standing in for this element's name
/// (html-aam's `figure-name-no-figcaption.tentative.html`, whose own comment
/// states the rule verbatim).
fn figure_figcaption_fallback(dom: &Dom, node: NodeIdx) -> Option<String> {
    let figure = dom.get(node).parent.filter(|&parent| dom.name(parent) == Some("figure"))?;
    let mut other_content = false;
    let mut figcaption_text = None;
    for child in dom.children(figure) {
        if child == node {
            continue;
        }
        match &dom.get(child).kind {
            NodeKind::Element { name, .. } if name.as_str() == "figcaption" => {
                figcaption_text = Some(host::child_text(dom, child));
            }
            NodeKind::Text { data } | NodeKind::CDATASection { data } => {
                if !data.trim().is_empty() {
                    other_content = true;
                }
            }
            _ => other_content = true,
        }
    }
    if other_content {
        return None;
    }
    figcaption_text.filter(|text| !text.trim().is_empty())
}

/// A node is hidden when it has `hidden`/`aria-hidden="true"`, or its computed
/// style is `display:none` / `visibility:hidden|collapse`.
pub fn is_hidden(ctx: &NameContext<'_>, node: NodeIdx) -> bool {
    let element = element_or_parent(ctx, node);
    let Some(element) = element else { return false };
    is_hard_hidden(ctx, element) || is_soft_hidden(ctx, element)
}

/// A text/CDATA node's containing element, or the node itself if it already
/// is one; `None` for anything else (comments, the document, …).
fn element_or_parent(ctx: &NameContext<'_>, node: NodeIdx) -> Option<NodeIdx> {
    match ctx.dom.get(node).kind {
        NodeKind::Element { .. } => Some(node),
        NodeKind::Text { .. } | NodeKind::CDATASection { .. } => ctx.dom.get(node).parent,
        _ => None,
    }
}

/// `display:none`, `[hidden]` and `aria-hidden="true"` hide the whole subtree
/// unconditionally — no descendant can override them.
fn is_hard_hidden(ctx: &NameContext<'_>, node: NodeIdx) -> bool {
    is_hard_hidden_raw(ctx.dom, ctx.styled, node)
}

/// [`is_hard_hidden`] without a [`NameContext`] — A11Y-13's
/// `tree::collect_owned_by` needs the same "is this element itself
/// unconditionally hidden" check (w3c/aria PR #1839, rule 1: `aria-owns` set
/// on an element excluded from the accessibility tree must not resolve) at a
/// point before a `NameContext` exists at all (it is one of the pieces that
/// eventually builds one).
pub(crate) fn is_hard_hidden_raw(dom: &Dom, styled: &StyledDom, node: NodeIdx) -> bool {
    if dom.get_attribute_ns(node, None, "hidden").is_some() {
        return true;
    }
    if dom
        .get_attribute_ns(node, None, "aria-hidden")
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("true"))
    {
        return true;
    }
    styled
        .get(node)
        .and_then(|style| style.get("display"))
        .is_some_and(|v| v.eq_ignore_ascii_case("none"))
}

/// `visibility:hidden`/`collapse`, which a descendant can reset back to
/// `visible` (an ordinary inherited-property override, already resolved in
/// each element's own computed style).
fn is_soft_hidden(ctx: &NameContext<'_>, node: NodeIdx) -> bool {
    ctx.styled.get(node).and_then(|style| style.get("visibility")).is_some_and(|v| {
        v.eq_ignore_ascii_case("hidden") || v.eq_ignore_ascii_case("collapse")
    })
}

/// Per accname: collapse runs of whitespace and trim. Only the HTML/accname
/// whitespace set (tab, LF, form feed, CR, space) counts — Unicode's broader
/// `char::is_whitespace` also matches U+00A0 NBSP, which accname explicitly
/// preserves as a significant character rather than a collapsible separator.
pub fn flatten(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut pending_space = false;
    for ch in value.chars() {
        if is_accname_whitespace(ch) {
            pending_space = !out.is_empty();
        } else {
            if pending_space {
                out.push(' ');
                pending_space = false;
            }
            out.push(ch);
        }
    }
    out
}

fn is_accname_whitespace(ch: char) -> bool {
    matches!(ch, '\t' | '\n' | '\u{0C}' | '\r' | ' ')
}

fn non_empty(value: Option<&str>) -> Option<String> {
    value.map(flatten).filter(|v| !v.is_empty())
}
