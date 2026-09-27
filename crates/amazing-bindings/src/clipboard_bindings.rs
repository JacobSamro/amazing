//! `navigator.clipboard` and `Clipboard.readText`/`writeText`, backed by an
//! in-memory clipboard (`Dom::clipboard_text`, DOM-B-REFLECT). Not a real OS
//! clipboard integration -- there is exactly one `Clipboard` singleton per
//! document arena, matching `Dom::clipboard_text`'s scope -- but enough for
//! same-page/same-session copy-paste round trips real pages exercise.
use crate::host::{BindingHost, InterfaceBrand, Operation as O, PlatformKey};

fn key<H: BindingHost>(host: &H) -> PlatformKey {
    PlatformKey { arena: host.node_platform_key(host.current_document()).arena, slot: 0, generation: 1 }
}
pub(crate) fn handles(op: O) -> bool {
    matches!(op, O::NavigatorClipboard | O::ClipboardReadText | O::ClipboardWriteText)
}
fn checked_clipboard<H: BindingHost>(host: &mut H, receiver: &H::Value) -> Result<(), H::Error> {
    if host.platform_object(receiver, InterfaceBrand::CLIPBOARD) == Some(key(host)) { Ok(()) }
    else { Err(host.type_error("Expected a Clipboard")) }
}
pub(crate) fn invoke<H: BindingHost>(host: &mut H, op: O, receiver: H::Value, args: &[H::Value])
    -> Result<H::Value, H::Error> {
    if op == O::NavigatorClipboard {
        return host.wrap_platform_object(key(host), InterfaceBrand::CLIPBOARD);
    }
    checked_clipboard(host, &receiver)?;
    match op {
        O::ClipboardReadText => {
            let text = host.with_dom(|dom| Ok(dom.clipboard_text().to_vec()))?;
            let promise = host.new_promise()?;
            let value = host.string(&text)?;
            host.resolve_promise(&promise, value)?;
            Ok(promise)
        }
        O::ClipboardWriteText => {
            let value = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
            let units = host.to_dom_string(value)?;
            host.with_dom(|dom| { dom.set_clipboard_text(units); Ok(()) })?;
            let promise = host.new_promise()?;
            let undefined = host.undefined();
            host.resolve_promise(&promise, undefined)?;
            Ok(promise)
        }
        _ => unreachable!("routed by handles()"),
    }
}
