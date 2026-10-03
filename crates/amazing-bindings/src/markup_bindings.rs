use crate::host::{BindingHost, FragmentParser, InterfaceBrand, Operation as O, SelectorEngine};
use amazing_dom::shadow::{ShadowMode, ShadowOptions, SlotAssignment};
use amazing_dom::{Dom, DomError, NodeIdx, NodeKind};

/// `Element`/`ShadowRoot.setHTMLUnsafe`'s top-level declarative-shadow-root
/// shape: `el.setHTMLUnsafe('<template shadowrootmode="open">...')`, where
/// the `<template>` is the fragment's very first (and typically only)
/// top-level content and is meant to attach directly to `el` itself, rather
/// than to some real ancestor already open within the fragment's own
/// markup (that nested case is handled entirely inside
/// `amazing_html::parse_fragment_with_shadow_roots` -- see its doc comment).
/// The parser has no real `NodeIdx` for the caller's actual host to attach
/// to, so it leaves this shape as an ordinary leftover `<template>`; the
/// caller (`invoke`, below) detects it and performs the attach itself.
/// Mirrors `amazing_html::tree::TreeBuilder::start_template`'s own reading
/// of the `shadowroot*` attributes for the nested case.
fn top_level_declarative_shadow_template(parsed: &Dom) -> Option<(NodeIdx, ShadowOptions, bool)> {
    let template = parsed.children(NodeIdx(0)).next()?;
    if parsed.namespace_uri(template) != Some("http://www.w3.org/1999/xhtml")
        || parsed.local_name(template) != Some("template") {
        return None;
    }
    let mode = match parsed.get_attribute_ns(template, None, "shadowrootmode")?.to_ascii_lowercase().as_str() {
        "open" => ShadowMode::Open,
        "closed" => ShadowMode::Closed,
        _ => return None,
    };
    let has = |name| parsed.get_attribute_ns(template, None, name).is_some();
    let manual = parsed.get_attribute_ns(template, None, "shadowrootslotassignment")
        .is_some_and(|value| value.eq_ignore_ascii_case("manual"));
    let options = ShadowOptions {
        mode, delegates_focus: has("shadowrootdelegatesfocus"),
        slot_assignment: if manual { SlotAssignment::Manual } else { SlotAssignment::Named },
        clonable: has("shadowrootclonable"), serializable: has("shadowrootserializable"),
    };
    Some((template, options, has("shadowrootcustomelementregistry")))
}

pub(crate) fn handles(op: O) -> bool {
    matches!(op, O::ElementGetHTML | O::ElementInnerHTML | O::ElementSetInnerHTML | O::ElementSetHTMLUnsafe | O::ElementOuterHTML
        | O::ElementSetOuterHTML | O::ElementInsertAdjacentHTML
        | O::ElementInsertAdjacentElement | O::ElementInsertAdjacentText | O::ElementInnerText
        | O::ElementSetInnerText | O::ParentQuerySelector | O::ParentQuerySelectorAll
        | O::ElementMatches | O::ElementClosest | O::ElementWebkitMatchesSelector)
}

fn required_string<H: BindingHost>(host: &mut H, arg: Option<&H::Value>) -> Result<Vec<u16>, H::Error> {
    let arg = arg.ok_or_else(|| host.type_error("Not enough arguments"))?;
    host.to_dom_string(arg)
}
fn seam_result<H: BindingHost, T>(host: &mut H, result: Result<T, DomError>) -> Result<T, H::Error> {
    match result { Ok(value) => Ok(value), Err(error) => host.with_dom(|_| Err(error)) }
}

pub(crate) fn invoke<H: BindingHost + FragmentParser + SelectorEngine>(
    host: &mut H, op: O, receiver: H::Value, args: &[H::Value],
) -> Result<H::Value, H::Error> {
    if matches!(op, O::ElementSetInnerHTML | O::ElementSetHTMLUnsafe | O::ElementSetOuterHTML
        | O::ElementInsertAdjacentHTML | O::ElementInsertAdjacentText | O::ElementSetInnerText) {
        host.current_document_for_nodes()?;
    }
    let node = host.node_brand(&receiver).ok_or_else(|| host.type_error("Expected a Node"))?;
    if matches!(op, O::ParentQuerySelector | O::ParentQuerySelectorAll | O::ElementMatches
        | O::ElementClosest | O::ElementWebkitMatchesSelector) {
        let element_only = !matches!(op, O::ParentQuerySelector | O::ParentQuerySelectorAll);
        host.with_dom(|dom| match &dom.get(node).kind {
            NodeKind::Element { .. } => Ok(()),
            NodeKind::Document | NodeKind::DocumentFragment if !element_only => Ok(()),
            _ => Err(DomError::TypeError),
        })?;
        let selector = required_string(host, args.first())?;
        let result = host.parse_selector(&selector);
        let parsed = seam_result(host, result)?;
        let candidates = host.with_dom(|dom| {
            if op == O::ElementClosest {
                Ok(dom.inclusive_ancestors(node).into_iter().filter(|id|
                    matches!(dom.get(*id).kind, NodeKind::Element { .. })).collect::<Vec<_>>())
            } else if matches!(op, O::ElementMatches | O::ElementWebkitMatchesSelector) {
                Ok(vec![node])
            } else {
                Ok(dom.tree_order(node).into_iter().skip(1).filter(|id|
                    matches!(dom.get(*id).kind, NodeKind::Element { .. })).collect::<Vec<_>>())
            }
        })?;
        let mut found = Vec::new();
        for candidate in candidates {
            let result = if op == O::ElementClosest {
                host.matches_selector_scoped(candidate, &parsed, node)
            } else {
                host.matches_selector(candidate, &parsed)
            };
            if seam_result(host, result)? {
                found.push(candidate);
                if op != O::ParentQuerySelectorAll { break; }
            }
        }
        return match op {
            O::ElementMatches | O::ElementWebkitMatchesSelector => Ok(host.boolean(!found.is_empty())),
            O::ParentQuerySelector | O::ElementClosest => match found.first() {
                Some(found) => host.wrap_node(*found), None => Ok(host.null()),
            },
            O::ParentQuerySelectorAll => {
                let slot = host.with_dom(|dom| Ok(dom.create_static_node_list(found)))?;
                let mut key = host.node_platform_key(host.current_document());
                key.slot = slot as u64;
                host.wrap_platform_object(key, InterfaceBrand::STATIC_NODE_LIST)
            }
            _ => unreachable!(),
        };
    }
    host.with_dom(|dom| if matches!(dom.get(node).kind, NodeKind::Element { .. })
        || (matches!(op, O::ElementGetHTML | O::ElementInnerHTML | O::ElementSetInnerHTML | O::ElementSetHTMLUnsafe)
            && dom.is_shadow_root(node)) { Ok(()) }
        else { Err(DomError::TypeError) })?;
    match op {
        O::ElementGetHTML => {
            let mut serializable = false;
            let mut roots = Vec::new();
            if let Some(options) = args.first().filter(|value| !host.is_nullish(value)) {
                let flag = host.get_property(options, "serializableShadowRoots")?;
                serializable = host.to_boolean(&flag);
                let list = host.get_property(options, "shadowRoots")?;
                if !host.is_nullish(&list) {
                    roots = host.sequence_shadow_roots(&list)?;
                    host.with_dom(|dom| if roots.iter().all(|root| dom.is_shadow_root(*root)) {
                        Ok(())
                    } else { Err(DomError::TypeError) })?;
                }
            }
            let units = host.with_dom(|dom| dom.serialize_children_with_shadow(node, serializable, &roots))?;
            host.string(&units)
        }
        O::ElementInnerHTML | O::ElementOuterHTML => {
            let units = host.with_dom(|dom| if op == O::ElementInnerHTML {
                dom.serialize_children(node)
            } else { dom.serialize_node(node) })?;
            host.string(&units)
        }
        O::ElementInnerText => {
            let units = host.with_dom(|dom| dom.inner_text_approx(node))?;
            host.string(&units)
        }
        O::ElementSetInnerText => {
            let value = required_string(host, args.first())?;
            host.with_dom(|dom| {
                let document = dom.owner_document(node).ok_or(DomError::InvalidStateError)?;
                let text = if value.is_empty() { None } else {
                    let text = dom.create_node_in(document, NodeKind::Text { data: String::new() })?;
                    dom.replace_character_data(text, 0, 0, &value)?;
                    Some(text)
                };
                dom.replace_all(node, text)
            })?;
            Ok(host.undefined())
        }
        O::ElementInsertAdjacentElement | O::ElementInsertAdjacentText => {
            let position = required_string(host, args.first())?;
            let position = String::from_utf16(&position).unwrap_or_default().to_ascii_lowercase();
            if !matches!(position.as_str(), "beforebegin" | "afterbegin" | "beforeend" | "afterend") {
                return host.with_dom::<H::Value>(|_| Err(DomError::SyntaxError));
            }
            let value = args.get(1).ok_or_else(|| host.type_error("Not enough arguments"))?;
            let element = if op == O::ElementInsertAdjacentElement {
                Some(host.node_brand(value).ok_or_else(|| host.type_error("Expected an Element"))?)
            } else { None };
            let text = if element.is_none() { Some(host.to_dom_string(value)?) } else { None };
            let inserted = host.with_dom(|dom| {
                if let Some(element) = element {
                    if !matches!(dom.get(element).kind, NodeKind::Element { .. }) {
                        return Err(DomError::TypeError);
                    }
                }
                let parent = dom.get(node).parent;
                let (parent, reference) = match position.as_str() {
                    "beforebegin" => (parent, Some(node)),
                    "afterend" => (parent, dom.get(node).next_sibling),
                    "afterbegin" => (Some(node), dom.get(node).first_child),
                    "beforeend" => (Some(node), None),
                    _ => unreachable!(),
                };
                let Some(parent) = parent else { return Ok(None) };
                let inserted = if let Some(element) = element { element } else {
                    let document = dom.owner_document(node).unwrap_or(node);
                    let text_node = dom.create_node_in(document, NodeKind::Text { data: String::new() })?;
                    dom.replace_character_data(text_node, 0, 0, text.as_deref().unwrap())?;
                    text_node
                };
                dom.pre_insert(inserted, parent, reference)?;
                Ok(Some(inserted))
            })?;
            if op == O::ElementInsertAdjacentText { return Ok(host.undefined()) }
            match inserted { Some(node) => host.wrap_node(node), None => Ok(host.null()) }
        }
        O::ElementSetInnerHTML | O::ElementSetHTMLUnsafe | O::ElementSetOuterHTML | O::ElementInsertAdjacentHTML => {
            let position = if op == O::ElementInsertAdjacentHTML {
                let units = required_string(host, args.first())?;
                let text = String::from_utf16(&units).unwrap_or_default().to_ascii_lowercase();
                if !matches!(text.as_str(), "beforebegin" | "afterbegin" | "beforeend" | "afterend") {
                    return host.with_dom::<H::Value>(|_| Err(DomError::SyntaxError));
                }
                Some(text)
            } else { None };
            let markup = required_string(host, args.get(if position.is_some() { 1 } else { 0 }))?;
            let (parent, reference, parse_context, context_node) = host.with_dom(|dom| {
                let parent = dom.get(node).parent;
                let (mut insertion_parent, reference) = match position.as_deref() {
                    Some("beforebegin") => (parent.ok_or(DomError::NoModificationAllowedError)?, Some(node)),
                    Some("afterend") => (parent.ok_or(DomError::NoModificationAllowedError)?, dom.get(node).next_sibling),
                    Some("afterbegin") => (node, dom.get(node).first_child),
                    Some("beforeend") => (node, None),
                    None if op == O::ElementSetOuterHTML => (parent.ok_or(DomError::NoModificationAllowedError)?, Some(node)),
                    None => (node, None),
                    _ => unreachable!(),
                };
                // Select the actual target before parsing: template setters
                // replace their persistent contents, whose document has no
                // browsing context. Keep its host for tokenizer/insertion mode.
                if matches!(op, O::ElementSetInnerHTML | O::ElementSetHTMLUnsafe) && dom.is_html_template(node) {
                    insertion_parent = dom.ensure_template_contents(node)?;
                }
                let context_node = if let Some(data) = dom.shadow_data(insertion_parent) {
                    if matches!(op, O::ElementSetInnerHTML | O::ElementSetHTMLUnsafe) { data.host } else { insertion_parent }
                } else if let Some(template) = dom.template_of_contents(insertion_parent) {
                    if matches!(op, O::ElementSetInnerHTML | O::ElementSetHTMLUnsafe) { template } else { insertion_parent }
                } else { insertion_parent };
                let context_document = dom.owner_document(insertion_parent).unwrap_or(insertion_parent);
                let element = matches!(dom.get(context_node).kind, NodeKind::Element { .. }).then_some(context_node);
                let context = crate::fragment_context::html(dom, element, context_document)?;
                Ok((insertion_parent, reference, context, context_node))
            })?;
            // `setHTMLUnsafe` alone parses with declarative shadow roots
            // enabled (spec's "set and filter HTML" always passes `true` for
            // `allowDeclarativeShadowRoots`, but only `setHTMLUnsafe`/
            // `parseHTML(Unsafe)` route through it) -- `innerHTML`/
            // `outerHTML`/`insertAdjacentHTML` keep them off, unchanged.
            let result = if op == O::ElementSetHTMLUnsafe {
                host.parse_fragment_with_shadow_roots(parse_context, &markup)
            } else {
                host.parse_fragment(parse_context, &markup)
            };
            let mut parsed = seam_result(host, result)?;
            // Nested declarative shadow roots (a real ancestor already open
            // within the fragment's own markup) survive into `parsed`
            // already, fully formed, courtesy of `parse_fragment_with_
            // shadow_roots`'s own shadow-aware cloning -- `import_fragment`
            // below imports them like any other node. Only the top-level
            // shape (the template meant to attach to `context_node` itself)
            // needs handling here; see `top_level_declarative_shadow_
            // template`'s doc comment for why the parser can't do it.
            let top_level_shadow = (op == O::ElementSetHTMLUnsafe)
                .then(|| top_level_declarative_shadow_template(&parsed)).flatten();
            host.with_dom(|dom| {
                let insertion = parent;
                let document = dom.owner_document(insertion).unwrap_or(insertion);
                if let Some((template, options, keep_registry_null)) = top_level_shadow {
                    // Spec's "attach a shadow root" throwing (already a
                    // shadow host, or `context_node`'s local name is not a
                    // valid declarative shadow host) is caught and the
                    // template is left as ordinary content -- do not detach
                    // it from `parsed` below, so the unmodified fallthrough
                    // `import_fragment` keeps it as a plain top-level child.
                    if let Ok(shadow_root) = dom.attach_declarative_shadow(context_node, options, keep_registry_null) {
                        if let Some(contents) = parsed.template_contents(template) {
                            dom.import_subtree_children(&parsed, contents, shadow_root)?;
                        }
                        parsed.pre_remove(template, NodeIdx(0)).expect("top-level template");
                    }
                }
                let fragment = dom.import_fragment(&parsed, document)?;
                if matches!(op, O::ElementSetInnerHTML | O::ElementSetHTMLUnsafe) { dom.replace_all(insertion, Some(fragment)) }
                else if op == O::ElementSetOuterHTML { dom.replace(node, fragment, parent).map(|_| ()) }
                else { dom.pre_insert(fragment, parent, reference).map(|_| ()) }
            })?;
            Ok(host.undefined())
        }
        _ => unreachable!(),
    }
}
