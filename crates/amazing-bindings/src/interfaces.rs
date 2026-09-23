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
const EVENT_TARGET: &[Member] = &[
    method("addEventListener", O::AddEventListener, 2),
    method("removeEventListener", O::RemoveEventListener, 2),
    method("dispatchEvent", O::DispatchEvent, 1),
];
const EVENT: &[Member] = &[
    getter("type", O::EventType),
    getter("target", O::EventTargetValue),
    getter("srcElement", O::EventTargetValue),
    getter("currentTarget", O::EventCurrentTarget),
    method("composedPath", O::EventComposedPath, 0),
    getter("eventPhase", O::EventPhase),
    getter("bubbles", O::EventBubbles),
    getter("cancelable", O::EventCancelable),
    getter("composed", O::EventComposed),
    getter("defaultPrevented", O::EventDefaultPrevented),
    getter("isTrusted", O::EventIsTrusted),
    getter("timeStamp", O::EventTimeStamp),
    getter("cancelBubble", O::EventCancelBubble),
    setter("cancelBubble", O::EventSetCancelBubble),
    getter("returnValue", O::EventReturnValue),
    setter("returnValue", O::EventSetReturnValue),
    method("stopPropagation", O::EventStopPropagation, 0),
    method("stopImmediatePropagation", O::EventStopImmediatePropagation, 0),
    method("preventDefault", O::EventPreventDefault, 0),
    method("initEvent", O::EventInitEvent, 1),
];
const CUSTOM_EVENT: &[Member] = &[
    getter("detail", O::CustomEventDetail),
    method("initCustomEvent", O::CustomEventInit, 1),
];
const MUTATION_OBSERVER: &[Member] = &[
    method("observe", O::MutationObserve, 2),
    method("disconnect", O::MutationDisconnect, 0),
    method("takeRecords", O::MutationTakeRecords, 0),
];
const MUTATION_RECORD: &[Member] = &[
    getter("type", O::MutationRecordType),
    getter("target", O::MutationRecordTarget),
    getter("addedNodes", O::MutationRecordAddedNodes),
    getter("removedNodes", O::MutationRecordRemovedNodes),
    getter("previousSibling", O::MutationRecordPreviousSibling),
    getter("nextSibling", O::MutationRecordNextSibling),
    getter("attributeName", O::MutationRecordAttributeName),
    getter("attributeNamespace", O::MutationRecordAttributeNamespace),
    getter("oldValue", O::MutationRecordOldValue),
];
const DOM_EXCEPTION: &[Member] = &[
    getter("name", O::DOMExceptionName),
    getter("message", O::DOMExceptionMessage),
    getter("code", O::DOMExceptionCode),
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
    getter("implementation", O::DocumentImplementation),
    method("createElement", O::CreateElement, 1),
    method("createElementNS", O::CreateElementNS, 2),
    method("createTextNode", O::CreateText, 1),
    method("createCDATASection", O::CreateCDATASection, 1),
    method("createAttribute", O::CreateAttribute, 1),
    method("createAttributeNS", O::CreateAttributeNS, 2),
    method("createProcessingInstruction", O::CreateProcessingInstruction, 2),
    method("createComment", O::CreateComment, 1),
    method("createDocumentFragment", O::CreateFragment, 0),
    method("createRange", O::CreateRange, 0),
    method("createNodeIterator", O::CreateNodeIterator, 1),
    method("createTreeWalker", O::CreateTreeWalker, 1),
    method("createEvent", O::CreateEvent, 1),
    method("append", O::Append, 0),
    method("prepend", O::Prepend, 0),
    method("replaceChildren", O::ReplaceChildren, 0),
];
const DOM_IMPLEMENTATION: &[Member] = &[
    method("createDocumentType", O::ImplementationCreateDocumentType, 3),
    method("createDocument", O::ImplementationCreateDocument, 3),
    method("createHTMLDocument", O::ImplementationCreateHTMLDocument, 1),
    method("hasFeature", O::ImplementationHasFeature, 0),
];
const NODE_LIST: &[Member] = &[
    getter("length", O::NodeListLength),
    method("item", O::NodeListItem, 1),
];
const ATTR: &[Member] = &[
    getter("namespaceURI", O::AttrNamespaceURI),
    getter("prefix", O::AttrPrefix),
    getter("localName", O::AttrLocalName),
    getter("name", O::AttrName),
    getter("value", O::AttrValue),
    setter("value", O::AttrSetValue),
    getter("ownerElement", O::AttrOwnerElement),
    getter("specified", O::AttrSpecified),
];
const NAMED_NODE_MAP: &[Member] = &[
    getter("length", O::NamedNodeMapLength),
    method("item", O::NamedNodeMapItem, 1),
    method("getNamedItem", O::NamedNodeMapGetNamedItem, 1),
    method("getNamedItemNS", O::NamedNodeMapGetNamedItemNS, 2),
    method("setNamedItem", O::NamedNodeMapSetNamedItem, 1),
    method("setNamedItemNS", O::NamedNodeMapSetNamedItemNS, 1),
    method("removeNamedItem", O::NamedNodeMapRemoveNamedItem, 1),
    method("removeNamedItemNS", O::NamedNodeMapRemoveNamedItemNS, 2),
];
const DOM_TOKEN_LIST: &[Member] = &[
    getter("length", O::TokenListLength),
    method("item", O::TokenListItem, 1),
    method("contains", O::TokenListContains, 1),
    method("add", O::TokenListAdd, 0),
    method("remove", O::TokenListRemove, 0),
    method("toggle", O::TokenListToggle, 1),
    method("replace", O::TokenListReplace, 2),
    method("supports", O::TokenListSupports, 1),
    getter("value", O::TokenListValue),
    setter("value", O::TokenListSetValue),
    method("toString", O::TokenListValue, 0),
];
const NODE_ITERATOR: &[Member] = &[
    getter("root", O::TraversalRoot),
    getter("referenceNode", O::IteratorReferenceNode),
    getter("pointerBeforeReferenceNode", O::IteratorPointerBeforeReferenceNode),
    getter("whatToShow", O::TraversalWhatToShow),
    getter("filter", O::TraversalFilter),
    method("nextNode", O::IteratorNextNode, 0),
    method("previousNode", O::IteratorPreviousNode, 0),
    method("detach", O::IteratorDetach, 0),
];
const TREE_WALKER: &[Member] = &[
    getter("root", O::TraversalRoot),
    getter("whatToShow", O::TraversalWhatToShow),
    getter("filter", O::TraversalFilter),
    getter("currentNode", O::WalkerCurrentNode),
    setter("currentNode", O::WalkerSetCurrentNode),
    method("parentNode", O::WalkerParentNode, 0),
    method("firstChild", O::WalkerFirstChild, 0),
    method("lastChild", O::WalkerLastChild, 0),
    method("previousSibling", O::WalkerPreviousSibling, 0),
    method("nextSibling", O::WalkerNextSibling, 0),
    method("previousNode", O::WalkerPreviousNode, 0),
    method("nextNode", O::WalkerNextNode, 0),
];
const ABSTRACT_RANGE: &[Member] = &[
    getter("startContainer", O::RangeStartContainer),
    getter("startOffset", O::RangeStartOffset),
    getter("endContainer", O::RangeEndContainer),
    getter("endOffset", O::RangeEndOffset),
    getter("collapsed", O::RangeCollapsed),
];
const RANGE: &[Member] = &[
    getter("commonAncestorContainer", O::RangeCommonAncestorContainer),
    method("setStart", O::RangeSetStart, 2),
    method("setEnd", O::RangeSetEnd, 2),
    method("setStartBefore", O::RangeSetStartBefore, 1),
    method("setStartAfter", O::RangeSetStartAfter, 1),
    method("setEndBefore", O::RangeSetEndBefore, 1),
    method("setEndAfter", O::RangeSetEndAfter, 1),
    method("collapse", O::RangeCollapse, 0),
    method("selectNode", O::RangeSelectNode, 1),
    method("selectNodeContents", O::RangeSelectNodeContents, 1),
    method("compareBoundaryPoints", O::RangeCompareBoundaryPoints, 2),
    method("deleteContents", O::RangeDeleteContents, 0),
    method("extractContents", O::RangeExtractContents, 0),
    method("cloneContents", O::RangeCloneContents, 0),
    method("insertNode", O::RangeInsertNode, 1),
    method("surroundContents", O::RangeSurroundContents, 1),
    method("cloneRange", O::RangeCloneRange, 0),
    method("detach", O::RangeDetach, 0),
    method("intersectsNode", O::RangeIntersectsNode, 1),
    method("isPointInRange", O::RangeIsPointInRange, 2),
    method("comparePoint", O::RangeComparePoint, 2),
    method("toString", O::RangeStringifier, 0),
];
const ELEMENT: &[Member] = &[
    getter("classList", O::ElementClassList),
    getter("className", O::ElementClassName),
    setter("className", O::ElementSetClassName),
    getter("attributes", O::ElementAttributes),
    method("getAttribute", O::ElementGetAttribute, 1),
    method("getAttributeNS", O::ElementGetAttributeNS, 2),
    method("setAttribute", O::ElementSetAttribute, 2),
    method("setAttributeNS", O::ElementSetAttributeNS, 3),
    method("removeAttribute", O::ElementRemoveAttribute, 1),
    method("removeAttributeNS", O::ElementRemoveAttributeNS, 2),
    method("getAttributeNode", O::ElementGetAttributeNode, 1),
    method("getAttributeNodeNS", O::ElementGetAttributeNodeNS, 2),
    method("setAttributeNode", O::ElementSetAttributeNode, 1),
    method("setAttributeNodeNS", O::ElementSetAttributeNodeNS, 1),
    method("removeAttributeNode", O::ElementRemoveAttributeNode, 1),
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
        interface: I::EventTarget,
        name: "EventTarget",
        parent: None,
        constructor: Some(O::EventTargetConstructor),
        members: EVENT_TARGET,
    },
    D {
        interface: I::Node,
        name: "Node",
        parent: Some(I::EventTarget),
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
        interface: I::CDATASection,
        name: "CDATASection",
        parent: Some(I::Text),
        constructor: None,
        members: &[],
    },
    D {
        interface: I::Attr,
        name: "Attr",
        parent: Some(I::Node),
        constructor: None,
        members: ATTR,
    },
    D {
        interface: I::NamedNodeMap,
        name: "NamedNodeMap",
        parent: None,
        constructor: None,
        members: NAMED_NODE_MAP,
    },
    D {
        interface: I::DOMTokenList,
        name: "DOMTokenList",
        parent: None,
        constructor: None,
        members: DOM_TOKEN_LIST,
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
    D {
        interface: I::AbstractRange,
        name: "AbstractRange",
        parent: None,
        constructor: None,
        members: ABSTRACT_RANGE,
    },
    D {
        interface: I::StaticRange,
        name: "StaticRange",
        parent: Some(I::AbstractRange),
        constructor: Some(O::StaticRangeConstructor),
        members: &[],
    },
    D {
        interface: I::Range,
        name: "Range",
        parent: Some(I::AbstractRange),
        constructor: Some(O::RangeConstructor),
        members: RANGE,
    },
    D {
        interface: I::DOMImplementation,
        name: "DOMImplementation",
        parent: None,
        constructor: None,
        members: DOM_IMPLEMENTATION,
    },
    D {
        interface: I::NodeFilter,
        name: "NodeFilter",
        parent: None,
        constructor: None,
        members: &[],
    },
    D {
        interface: I::NodeList,
        name: "NodeList",
        parent: None,
        constructor: None,
        members: NODE_LIST,
    },
    D {
        interface: I::NodeIterator,
        name: "NodeIterator",
        parent: None,
        constructor: None,
        members: NODE_ITERATOR,
    },
    D {
        interface: I::TreeWalker,
        name: "TreeWalker",
        parent: None,
        constructor: None,
        members: TREE_WALKER,
    },
    D {
        interface: I::Event,
        name: "Event",
        parent: None,
        constructor: Some(O::EventConstructor),
        members: EVENT,
    },
    D {
        interface: I::CustomEvent,
        name: "CustomEvent",
        parent: Some(I::Event),
        constructor: Some(O::CustomEventConstructor),
        members: CUSTOM_EVENT,
    },
    D {
        interface: I::MutationObserver,
        name: "MutationObserver",
        parent: None,
        constructor: Some(O::MutationObserverConstructor),
        members: MUTATION_OBSERVER,
    },
    D {
        interface: I::MutationRecord,
        name: "MutationRecord",
        parent: None,
        constructor: None,
        members: MUTATION_RECORD,
    },
    D {
        interface: I::DOMException,
        name: "DOMException",
        parent: None,
        constructor: Some(O::DOMExceptionConstructor),
        members: DOM_EXCEPTION,
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
    for (name, value) in [
        ("START_TO_START", 0),
        ("START_TO_END", 1),
        ("END_TO_END", 2),
        ("END_TO_START", 3),
    ] {
        host.install_interface_constant(I::Range, name, value)?;
    }
    for (name, value) in [
        ("FILTER_ACCEPT", 1), ("FILTER_REJECT", 2), ("FILTER_SKIP", 3),
        ("SHOW_ALL", u32::MAX), ("SHOW_ELEMENT", 1), ("SHOW_ATTRIBUTE", 2),
        ("SHOW_TEXT", 4), ("SHOW_CDATA_SECTION", 8),
        ("SHOW_ENTITY_REFERENCE", 16), ("SHOW_ENTITY", 32),
        ("SHOW_PROCESSING_INSTRUCTION", 64), ("SHOW_COMMENT", 128),
        ("SHOW_DOCUMENT", 256), ("SHOW_DOCUMENT_TYPE", 512),
        ("SHOW_DOCUMENT_FRAGMENT", 1024), ("SHOW_NOTATION", 2048),
    ] {
        host.install_interface_constant(I::NodeFilter, name, value)?;
    }
    for (name, value) in [("NONE", 0), ("CAPTURING_PHASE", 1), ("AT_TARGET", 2), ("BUBBLING_PHASE", 3)] {
        host.install_interface_constant(I::Event, name, value)?;
    }
    for (name, value) in crate::dom_exception::LEGACY_CODES {
        host.install_interface_constant(I::DOMException, name, u32::from(*value))?;
    }
    Ok(())
}
