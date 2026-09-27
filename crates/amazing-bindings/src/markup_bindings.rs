use crate::host::{BindingHost, FragmentContext, FragmentParser, InterfaceBrand, Operation as O, SelectorEngine};
use amazing_dom::{DomError, NodeKind};

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
            let (parent, reference, parse_context) = host.with_dom(|dom| {
                let parent = dom.get(node).parent;
                let (insertion_parent, reference) = match position.as_deref() {
                    Some("beforebegin") => (parent.ok_or(DomError::NoModificationAllowedError)?, Some(node)),
                    Some("afterend") => (parent.ok_or(DomError::NoModificationAllowedError)?, dom.get(node).next_sibling),
                    Some("afterbegin") => (node, dom.get(node).first_child),
                    Some("beforeend") => (node, None),
                    None if op == O::ElementSetOuterHTML => (parent.ok_or(DomError::NoModificationAllowedError)?, Some(node)),
                    None => (node, None),
                    _ => unreachable!(),
                };
                let context_node = if insertion_parent == node { node } else { insertion_parent };
                let context_node = dom.shadow_data(context_node).map_or(context_node, |data| data.host);
                let context = if matches!(dom.get(context_node).kind, NodeKind::Document) {
                    FragmentContext { local_name: "body".into(), namespace_uri: Some("http://www.w3.org/1999/xhtml".into()) }
                } else {
                    FragmentContext { local_name: dom.local_name(context_node).ok_or(DomError::TypeError)?.into(),
                        namespace_uri: dom.namespace_uri(context_node).map(str::to_owned) }
                };
                Ok((insertion_parent, reference, context))
            })?;
            let result = host.parse_fragment(parse_context, &markup);
            let parsed = seam_result(host, result)?;
            host.with_dom(|dom| {
                let insertion = if matches!(op, O::ElementSetInnerHTML | O::ElementSetHTMLUnsafe) && dom.is_html_template(node) {
                    dom.ensure_template_contents(node)?
                } else { parent };
                let document = dom.owner_document(insertion).unwrap_or(insertion);
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
