use crate::host::{
    BindingHost, Interface as I, InterfaceDefinition as D, Member, MemberKind as K, Operation as O,
};
const fn method(name: &'static str, operation: O, length: u8) -> Member {
    Member {
        name,
        operation,
        kind: K::Method { length },
    }
}
const fn getter(name: &'static str, operation: O) -> Member {
    Member {
        name,
        operation,
        kind: K::Getter,
    }
}
const fn setter(name: &'static str, operation: O) -> Member {
    Member {
        name,
        operation,
        kind: K::Setter,
    }
}
const NODE: &[Member] = &[
    getter("nodeType", O::NodeType),
    getter("nodeName", O::NodeName),
    getter("nodeValue", O::NodeValue),
    setter("nodeValue", O::SetNodeValue),
    getter("textContent", O::TextContent),
    setter("textContent", O::SetTextContent),
    getter("parentNode", O::ParentNode),
    getter("parentElement", O::ParentElement),
    getter("ownerDocument", O::OwnerDocument),
    getter("firstChild", O::FirstChild),
    getter("lastChild", O::LastChild),
    getter("previousSibling", O::PreviousSibling),
    getter("nextSibling", O::NextSibling),
    getter("childNodes", O::ChildNodes),
    getter("isConnected", O::IsConnected),
    method("hasChildNodes", O::HasChildNodes, 0),
    method("contains", O::Contains, 1),
    method("appendChild", O::AppendChild, 1),
    method("insertBefore", O::InsertBefore, 2),
    method("removeChild", O::RemoveChild, 1),
    method("replaceChild", O::ReplaceChild, 2),
    method("cloneNode", O::CloneNode, 0),
];
const PARENT: &[Member] = &[
    method("append", O::Append, 0),
    method("prepend", O::Prepend, 0),
    method("replaceChildren", O::ReplaceChildren, 0),
];
const CHILD: &[Member] = &[
    method("before", O::Before, 0),
    method("after", O::After, 0),
    method("replaceWith", O::ReplaceWith, 0),
    method("remove", O::Remove, 0),
];
const CHARACTER_DATA: &[Member] = &[
    getter("data", O::Data),
    setter("data", O::SetData),
    getter("length", O::Length),
    method("before", O::Before, 0),
    method("after", O::After, 0),
    method("replaceWith", O::ReplaceWith, 0),
    method("remove", O::Remove, 0),
];
const DOCUMENT: &[Member] = &[
    method("createTextNode", O::CreateText, 1),
    method("createComment", O::CreateComment, 1),
    method("createDocumentFragment", O::CreateFragment, 0),
    method("append", O::Append, 0),
    method("prepend", O::Prepend, 0),
    method("replaceChildren", O::ReplaceChildren, 0),
];
const ELEMENT: &[Member] = &[
    method("append", O::Append, 0),
    method("prepend", O::Prepend, 0),
    method("replaceChildren", O::ReplaceChildren, 0),
    method("before", O::Before, 0),
    method("after", O::After, 0),
    method("replaceWith", O::ReplaceWith, 0),
    method("remove", O::Remove, 0),
];
pub static INTERFACES: &[D] = &[
    D {
        interface: I::Node,
        name: "Node",
        parent: None,
        constructor: None,
        members: NODE,
    },
    D {
        interface: I::Document,
        name: "Document",
        parent: Some(I::Node),
        constructor: Some(O::DocumentConstructor),
        members: DOCUMENT,
    },
    D {
        interface: I::DocumentFragment,
        name: "DocumentFragment",
        parent: Some(I::Node),
        constructor: Some(O::FragmentConstructor),
        members: PARENT,
    },
    D {
        interface: I::DocumentType,
        name: "DocumentType",
        parent: Some(I::Node),
        constructor: None,
        members: CHILD,
    },
    D {
        interface: I::Element,
        name: "Element",
        parent: Some(I::Node),
        constructor: None,
        members: ELEMENT,
    },
    D {
        interface: I::CharacterData,
        name: "CharacterData",
        parent: Some(I::Node),
        constructor: None,
        members: CHARACTER_DATA,
    },
    D {
        interface: I::Text,
        name: "Text",
        parent: Some(I::CharacterData),
        constructor: Some(O::TextConstructor),
        members: &[],
    },
    D {
        interface: I::Comment,
        name: "Comment",
        parent: Some(I::CharacterData),
        constructor: Some(O::CommentConstructor),
        members: &[],
    },
    D {
        interface: I::ProcessingInstruction,
        name: "ProcessingInstruction",
        parent: Some(I::CharacterData),
        constructor: None,
        members: &[],
    },
];

/// Add these descriptors without deleting unrelated existing browser APIs.
/// Constructors require `new`; illegal constructors still expose prototypes.
/// Runtime defines writable/configurable methods, enumerable configurable
/// accessors, constructor/prototype links, and the proper function lengths.
pub fn install<H: BindingHost>(host: &mut H) -> Result<(), H::Error> {
    for definition in INTERFACES {
        host.install_interface(definition)?;
    }
    for (name, value) in [
        ("ELEMENT_NODE", 1),
        ("ATTRIBUTE_NODE", 2),
        ("TEXT_NODE", 3),
        ("CDATA_SECTION_NODE", 4),
        ("ENTITY_REFERENCE_NODE", 5),
        ("ENTITY_NODE", 6),
        ("PROCESSING_INSTRUCTION_NODE", 7),
        ("COMMENT_NODE", 8),
        ("DOCUMENT_NODE", 9),
        ("DOCUMENT_TYPE_NODE", 10),
        ("DOCUMENT_FRAGMENT_NODE", 11),
        ("NOTATION_NODE", 12),
        ("DOCUMENT_POSITION_DISCONNECTED", 1),
        ("DOCUMENT_POSITION_PRECEDING", 2),
        ("DOCUMENT_POSITION_FOLLOWING", 4),
        ("DOCUMENT_POSITION_CONTAINS", 8),
        ("DOCUMENT_POSITION_CONTAINED_BY", 16),
        ("DOCUMENT_POSITION_IMPLEMENTATION_SPECIFIC", 32),
    ] {
        host.install_node_constant(name, value)?;
    }
    Ok(())
}
