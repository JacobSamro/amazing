//! Loader-backed HTMLImageElement state. Attribute reflection lives in
//! element_reflection; the loader owns current request and decode completion.
use crate::host::{BindingHost, Operation as O, OptionalHookInstall};
use amazing_dom::{DomError, Namespace, NodeKind};

pub(crate) fn handles(op: O) -> bool {
    matches!(op, O::ImageCurrentSrc | O::ImageComplete | O::ImageNaturalWidth
        | O::ImageNaturalHeight | O::ImageDecode)
}

pub(crate) fn invoke<H: BindingHost>(
    host: &mut H, op: O, receiver: H::Value,
) -> Result<H::Value, H::Error> {
    let node = host.node_brand(&receiver).ok_or_else(|| host.type_error("Expected an HTMLImageElement"))?;
    host.with_dom(|dom| match &dom.get(node).kind {
        NodeKind::Element { namespace: Namespace::Html, .. } if dom.local_name(node) == Some("img") => Ok(()),
        _ => Err(DomError::TypeError),
    })?;
    if op == O::ImageDecode {
        let promise = host.new_promise()?;
        if host.image_decode(node, &promise)? == OptionalHookInstall::NotSupported {
            let reason = host.dom_exception_value(DomError::NotSupportedError)?;
            host.reject_promise(&promise, reason)?;
        }
        return Ok(promise);
    }
    let state = host.image_loader_state(node)?;
    Ok(match op {
        O::ImageCurrentSrc => host.string(&state.as_ref().map_or(&[][..], |state| &state.current_src))?,
        O::ImageNaturalWidth => host.number(state.as_ref().map_or(0, |state| state.natural_width) as f64),
        O::ImageNaturalHeight => host.number(state.as_ref().map_or(0, |state| state.natural_height) as f64),
        O::ImageComplete => {
            let fallback = host.with_dom(|dom| {
                let src = dom.attribute_utf16(node, "src");
                let srcset = dom.attribute_utf16(node, "srcset");
                Ok(srcset.is_none() && src.as_ref().is_none_or(Vec::is_empty))
            })?;
            host.boolean(state.as_ref().map_or(fallback, |state| state.complete))
        }
        _ => unreachable!(),
    })
}
