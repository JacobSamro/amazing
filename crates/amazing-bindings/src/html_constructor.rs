//! HTML §3.2.3 constructor steps. Author getters run outside native borrows.
use crate::host::{BindingHost, Interface, Operation};
use amazing_dom::{events::CallbackId, render_mutation::NodeGeneration, DomError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HtmlConstructorIdentity {
    pub document: NodeGeneration,
    pub creation_document: NodeGeneration,
    pub definition: usize,
    pub constructor: CallbackId,
}

/// The embedding captures the active constructor's callee realm registry,
/// matches actual newTarget identity, and retains its original native arena.
/// Capture rejects active-function equality, foreign/unregistered constructors,
/// and interfaces without [HTMLConstructor]. No author code runs in capture.
pub trait HtmlConstructorHost: BindingHost {
    type HtmlConstructorCapture;
    fn capture_html_constructor(&mut self, interface: Interface, new_target: &Self::Value)
        -> Result<Self::HtmlConstructorCapture, Self::Error>;
    fn html_constructor_identity(&self, capture: &Self::HtmlConstructorCapture) -> HtmlConstructorIdentity;
    fn html_constructor_dom_capture(&self, capture: &Self::HtmlConstructorCapture) -> Self::DomCapture;
    /// Actual GetFunctionRealm(newTarget) intrinsic, including foreign realms.
    fn html_constructor_intrinsic_prototype(&mut self, capture: &Self::HtmlConstructorCapture)
        -> Result<Self::Value, Self::Error>;
    /// Canonical wrapper in the captured arena; performs real [[SetPrototypeOf]]
    /// and propagates failure. Never constructs an unrelated ordinary object.
    fn html_constructor_wrap(&mut self, capture: &Self::HtmlConstructorCapture,
        element: NodeGeneration, prototype: &Self::Value) -> Result<Self::Value, Self::Error>;
}

pub fn invoke_html_constructor<H: HtmlConstructorHost>(host: &mut H, interface: Interface,
    new_target: Option<H::Value>) -> Result<H::Value, H::Error> {
    let Some(new_target) = new_target else { return Err(host.type_error("Illegal constructor")); };
    if !crate::INTERFACES.iter().any(|definition| definition.interface == interface
        && definition.constructor == Some(Operation::HTMLConstructor(interface))) {
        return Err(host.type_error("Interface has no HTML constructor"));
    }
    let capture = host.capture_html_constructor(interface, &new_target)?;
    let identity = host.html_constructor_identity(&capture);
    let dom_capture = host.html_constructor_dom_capture(&capture);
    host.with_captured_dom(&dom_capture, |dom| {
        let definition = dom.html_constructor_definition(identity.document, identity.definition, identity.constructor)?;
        let valid = if definition.name == definition.local_name {
            interface == Interface::HTMLElement
        } else {
            crate::html_element_interface(&definition.local_name) == interface
                && interface != Interface::HTMLUnknownElement
        };
        if valid { Ok(()) } else { Err(DomError::TypeError) }
    })?;
    let mut prototype = host.get_property(&new_target, "prototype")?;
    if !host.is_object_value(&prototype)? {
        prototype = host.html_constructor_intrinsic_prototype(&capture)?;
    }
    // Getter reentrancy may legitimately change the stack. Issue authority only
    // now, while retaining the original arena and definition captured above.
    let capability = host.with_captured_dom(&dom_capture, |dom|
        dom.prepare_html_constructor_in_document(identity.document, identity.creation_document, identity.definition, identity.constructor))?;
    let wrapper = host.html_constructor_wrap(&capture, capability.element(), &prototype)?;
    host.with_captured_dom(&dom_capture, |dom| dom.consume_html_constructor(capability))?;
    Ok(wrapper)
}
