//! DOMException names and historical `code` values owned by the binding layer.
use amazing_dom::DomError;
use crate::host::{BindingHost, InterfaceBrand, Operation as O, PlatformKey};

/// The name and legacy code for a native DOM failure. JS TypeError is distinct.
pub fn from_dom_error(error: DomError) -> Option<(&'static str, u16)> {
    Some(match error {
        DomError::HierarchyRequestError => ("HierarchyRequestError", 3),
        DomError::WrongDocumentError => ("WrongDocumentError", 4),
        DomError::InvalidCharacterError => ("InvalidCharacterError", 5),
        DomError::NotFoundError => ("NotFoundError", 8),
        DomError::NotSupportedError => ("NotSupportedError", 9),
        DomError::InvalidStateError => ("InvalidStateError", 11),
        DomError::NamespaceError => ("NamespaceError", 14),
        DomError::InUseAttributeError => ("InUseAttributeError", 10),
        DomError::SyntaxError => ("SyntaxError", 12),
        DomError::NoModificationAllowedError => ("NoModificationAllowedError", 7),
        DomError::InvalidNodeTypeError => ("InvalidNodeTypeError", 24),
        DomError::IndexSizeError => ("IndexSizeError", 1),
        DomError::TypeError | DomError::InvalidNode => return None,
    })
}

pub const LEGACY_CODES: &[(&str, u16)] = &[
    ("INDEX_SIZE_ERR", 1), ("DOMSTRING_SIZE_ERR", 2),
    ("HIERARCHY_REQUEST_ERR", 3), ("WRONG_DOCUMENT_ERR", 4),
    ("INVALID_CHARACTER_ERR", 5), ("NO_DATA_ALLOWED_ERR", 6),
    ("NO_MODIFICATION_ALLOWED_ERR", 7), ("NOT_FOUND_ERR", 8),
    ("NOT_SUPPORTED_ERR", 9), ("INUSE_ATTRIBUTE_ERR", 10),
    ("INVALID_STATE_ERR", 11), ("SYNTAX_ERR", 12),
    ("INVALID_MODIFICATION_ERR", 13), ("NAMESPACE_ERR", 14),
    ("INVALID_ACCESS_ERR", 15), ("VALIDATION_ERR", 16),
    ("TYPE_MISMATCH_ERR", 17), ("SECURITY_ERR", 18),
    ("NETWORK_ERR", 19), ("ABORT_ERR", 20),
    ("URL_MISMATCH_ERR", 21), ("QUOTA_EXCEEDED_ERR", 22),
    ("TIMEOUT_ERR", 23), ("INVALID_NODE_TYPE_ERR", 24),
    ("DATA_CLONE_ERR", 25),
];

pub fn legacy_code(name: &[u16]) -> u16 {
    let Ok(name) = String::from_utf16(name) else { return 0 };
    match name.as_str() {
        "IndexSizeError" => 1,
        "HierarchyRequestError" => 3,
        "WrongDocumentError" => 4,
        "InvalidCharacterError" => 5,
        "NoModificationAllowedError" => 7,
        "NotFoundError" => 8,
        "NotSupportedError" => 9,
        "InUseAttributeError" => 10,
        "InvalidStateError" => 11,
        "SyntaxError" => 12,
        "InvalidModificationError" => 13,
        "NamespaceError" => 14,
        "InvalidAccessError" => 15,
        "TypeMismatchError" => 17,
        "SecurityError" => 18,
        "NetworkError" => 19,
        "AbortError" => 20,
        "URLMismatchError" => 21,
        "QuotaExceededError" => 22,
        "TimeoutError" => 23,
        "InvalidNodeTypeError" => 24,
        "DataCloneError" => 25,
        _ => 0,
    }
}

pub(crate) fn handles(op: O) -> bool {
    matches!(op, O::DOMExceptionConstructor | O::DOMExceptionName
        | O::DOMExceptionMessage | O::DOMExceptionCode)
}
fn key<H: BindingHost>(host: &H, slot: usize) -> PlatformKey {
    PlatformKey { arena: host.node_platform_key(host.current_document()).arena,
        slot: slot as u64, generation: 1 }
}
pub(crate) fn create<H: BindingHost>(host: &mut H, name: Vec<u16>, message: Vec<u16>) -> Result<H::Value, H::Error> {
    let slot = host.with_dom(|dom| Ok(dom.create_dom_exception(name, message)))?;
    host.wrap_platform_object(key(host, slot), InterfaceBrand::DOM_EXCEPTION)
}
pub fn throw_dom_error<H: BindingHost>(host: &mut H, error: DomError) -> H::Error {
    let Some((name, _)) = from_dom_error(error) else { return host.type_error("Invalid DOM operation") };
    match create(host, name.encode_utf16().collect(), error.to_string().encode_utf16().collect()) {
        Ok(value) => host.throw_value(value),
        Err(error) => error,
    }
}
pub(crate) fn invoke<H: BindingHost>(
    host: &mut H, op: O, receiver: H::Value, args: &[H::Value],
) -> Result<H::Value, H::Error> {
    if op == O::DOMExceptionConstructor {
        // Optional arguments with defaults treat explicit undefined as omitted.
        // Convert message before name; either coercion may execute user code.
        let message = match args.first().filter(|value| !host.is_undefined(value)) {
            Some(value) => host.to_dom_string(value)?, None => Vec::new(),
        };
        let name = match args.get(1).filter(|value| !host.is_undefined(value)) {
            Some(value) => host.to_dom_string(value)?, None => "Error".encode_utf16().collect(),
        };
        return create(host, name, message);
    }
    let key = host.platform_object(&receiver, InterfaceBrand::DOM_EXCEPTION)
        .ok_or_else(|| host.type_error("Expected a DOMException"))?;
    if key.arena != host.node_platform_key(host.current_document()).arena || key.generation != 1 {
        return Err(host.type_error("Invalid DOMException"));
    }
    let data = host.with_dom(|dom| dom.dom_exception(key.slot as usize).cloned()
        .ok_or(DomError::InvalidStateError))?;
    Ok(match op {
        O::DOMExceptionName => host.string(&data.name)?,
        O::DOMExceptionMessage => host.string(&data.message)?,
        O::DOMExceptionCode => host.number(f64::from(legacy_code(&data.name))),
        _ => unreachable!(),
    })
}
