use crate::host::{
    BindingHost, Interface as I, InterfaceBrand, InterfaceDefinition as D, IterableKind, LegacyPlatformHooks, Member, MemberKind as K, Operation as O, OptionalHookInstall,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionalCapability {
    InterfaceClassString(I),
    DomExceptionErrorPrototype,
    CustomElementsGlobal,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct InstallReport {
    pub not_installed: Vec<OptionalCapability>,
}
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
    getter("baseURI", O::NodeBaseURI),
    method("hasChildNodes", O::HasChildNodes, 0),
    method("contains", O::Contains, 1),
    method("appendChild", O::AppendChild, 1),
    method("insertBefore", O::InsertBefore, 2),
    method("removeChild", O::RemoveChild, 1),
    method("replaceChild", O::ReplaceChild, 2),
    method("cloneNode", O::CloneNode, 0),
    method("getRootNode", O::GetRootNode, 0),
];
const EVENT_TARGET: &[Member] = &[
    method("addEventListener", O::AddEventListener, 2),
    method("removeEventListener", O::RemoveEventListener, 2),
    method("dispatchEvent", O::DispatchEvent, 1),
];
const ABORT_CONTROLLER: &[Member] = &[
    getter("signal", O::AbortControllerSignal),
    method("abort", O::AbortControllerAbort, 0),
];
const ABORT_SIGNAL: &[Member] = &[
    getter("aborted", O::AbortSignalAborted),
    getter("reason", O::AbortSignalReason),
    method("throwIfAborted", O::AbortSignalThrowIfAborted, 0),
    getter("onabort", O::AbortSignalOnAbort), setter("onabort", O::AbortSignalSetOnAbort),
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
const CUSTOM_ELEMENT_REGISTRY: &[Member] = &[
    method("define", O::CustomElementRegistryDefine, 2),
    method("get", O::CustomElementRegistryGet, 1),
    method("getName", O::CustomElementRegistryGetName, 1),
    method("whenDefined", O::CustomElementRegistryWhenDefined, 1),
    method("upgrade", O::CustomElementRegistryUpgrade, 1),
];
const PARENT: &[Member] = &[
    method("getElementById", O::FragmentGetElementById, 1),
    method("querySelector", O::ParentQuerySelector, 1),
    method("querySelectorAll", O::ParentQuerySelectorAll, 1),
    getter("children", O::FragmentChildren),
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
    getter("title", O::DocumentTitle), setter("title", O::DocumentSetTitle),
    getter("head", O::DocumentHead),
    getter("body", O::DocumentBody), setter("body", O::DocumentSetBody),
    getter("documentElement", O::DocumentElement),
    getter("doctype", O::DocumentDoctype),
    getter("readyState", O::DocumentReadyState),
    getter("URL", O::DocumentURL), getter("documentURI", O::DocumentURI),
    getter("characterSet", O::DocumentCharacterSet), getter("contentType", O::DocumentContentType),
    getter("compatMode", O::DocumentCompatMode), getter("referrer", O::DocumentReferrer),
    method("getElementById", O::DocumentGetElementById, 1),
    method("getElementsByName", O::DocumentGetElementsByName, 1),
    getter("forms", O::DocumentForms), getter("images", O::DocumentImages),
    getter("links", O::DocumentLinks), getter("scripts", O::DocumentScripts),
    getter("activeElement", O::DocumentActiveElement),
    method("hasFocus", O::DocumentHasFocus, 0),
    method("querySelector", O::ParentQuerySelector, 1),
    method("querySelectorAll", O::ParentQuerySelectorAll, 1),
    getter("children", O::DocumentChildren),
    method("getElementsByTagName", O::DocumentGetElementsByTagName, 1),
    method("getElementsByTagNameNS", O::DocumentGetElementsByTagNameNS, 2),
    method("getElementsByClassName", O::DocumentGetElementsByClassName, 1),
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
const HTML_COLLECTION: &[Member] = &[
    getter("length", O::HTMLCollectionLength),
    method("item", O::HTMLCollectionItem, 1),
    method("namedItem", O::HTMLCollectionNamedItem, 1),
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
    method("attachShadow", O::ElementAttachShadow, 1),
    getter("shadowRoot", O::ElementShadowRoot),
    getter("assignedSlot", O::ElementAssignedSlot),
    getter("slot", O::ElementSlot), setter("slot", O::ElementSetSlot),
    getter("innerHTML", O::ElementInnerHTML), setter("innerHTML", O::ElementSetInnerHTML),
    getter("outerHTML", O::ElementOuterHTML), setter("outerHTML", O::ElementSetOuterHTML),
    method("insertAdjacentHTML", O::ElementInsertAdjacentHTML, 2),
    method("querySelector", O::ParentQuerySelector, 1),
    method("querySelectorAll", O::ParentQuerySelectorAll, 1),
    method("matches", O::ElementMatches, 1),
    method("closest", O::ElementClosest, 1),
    method("webkitMatchesSelector", O::ElementWebkitMatchesSelector, 1),
    getter("id", O::ElementId),
    setter("id", O::ElementSetId),
    getter("children", O::ElementChildren),
    method("getElementsByTagName", O::ElementGetElementsByTagName, 1),
    method("getElementsByTagNameNS", O::ElementGetElementsByTagNameNS, 2),
    method("getElementsByClassName", O::ElementGetElementsByClassName, 1),
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
const SHADOW_ROOT: &[Member] = &[
    getter("mode", O::ShadowMode),
    getter("host", O::ShadowHost),
    getter("delegatesFocus", O::ShadowDelegatesFocus),
    getter("slotAssignment", O::ShadowSlotAssignment),
    getter("clonable", O::ShadowClonable),
    getter("serializable", O::ShadowSerializable),
    getter("innerHTML", O::ElementInnerHTML), setter("innerHTML", O::ElementSetInnerHTML),
];
const SLOT: &[Member] = &[
    getter("name", O::SlotName), setter("name", O::SlotSetName),
    method("assign", O::SlotAssign, 0),
    method("assignedNodes", O::SlotAssignedNodes, 0),
    method("assignedElements", O::SlotAssignedElements, 0),
];
const TEXT: &[Member] = &[getter("assignedSlot", O::TextAssignedSlot)];
const HTML_ELEMENT: &[Member] = &[
    getter("innerText", O::ElementInnerText), setter("innerText", O::ElementSetInnerText),
    getter("title", O::HTMLTitle), setter("title", O::HTMLSetTitle),
    getter("lang", O::HTMLLang), setter("lang", O::HTMLSetLang),
    getter("dir", O::HTMLDir), setter("dir", O::HTMLSetDir),
    getter("hidden", O::HTMLHidden), setter("hidden", O::HTMLSetHidden),
    getter("tabIndex", O::HTMLTabIndex), setter("tabIndex", O::HTMLSetTabIndex),
    getter("accessKey", O::HTMLAccessKey), setter("accessKey", O::HTMLSetAccessKey),
    getter("translate", O::HTMLTranslate), setter("translate", O::HTMLSetTranslate),
];
const HYPERLINK: &[Member] = &[
    getter("href", O::ElementHref), setter("href", O::ElementSetHref),
    method("toString", O::ElementHref, 0),
    getter("target", O::ElementTarget), setter("target", O::ElementSetTarget),
    getter("rel", O::ElementRel), setter("rel", O::ElementSetRel),
    getter("relList", O::ElementRelList),
    getter("download", O::ElementDownload), setter("download", O::ElementSetDownload),
];
const IMAGE: &[Member] = &[
    getter("src", O::ElementSrc), setter("src", O::ElementSetSrc),
    getter("alt", O::ImageAlt), setter("alt", O::ImageSetAlt),
    getter("width", O::ImageWidth), setter("width", O::ImageSetWidth),
    getter("height", O::ImageHeight), setter("height", O::ImageSetHeight),
    getter("loading", O::ImageLoading), setter("loading", O::ImageSetLoading),
];
const INPUT: &[Member] = &[
    getter("type", O::InputType), setter("type", O::InputSetType),
    getter("name", O::ElementName), setter("name", O::ElementSetName),
    getter("disabled", O::ElementDisabled), setter("disabled", O::ElementSetDisabled),
    getter("placeholder", O::InputPlaceholder), setter("placeholder", O::InputSetPlaceholder),
];
const SCRIPT: &[Member] = &[
    getter("src", O::ElementSrc), setter("src", O::ElementSetSrc),
    getter("type", O::ResourceType), setter("type", O::ResourceSetType),
    getter("async", O::ScriptAsync), setter("async", O::ScriptSetAsync),
    getter("defer", O::ScriptDefer), setter("defer", O::ScriptSetDefer),
];
const LINK: &[Member] = &[
    getter("href", O::ElementHref), setter("href", O::ElementSetHref),
    getter("rel", O::ElementRel), setter("rel", O::ElementSetRel),
    getter("relList", O::ElementRelList),
    getter("media", O::LinkMedia), setter("media", O::LinkSetMedia),
    getter("type", O::ResourceType), setter("type", O::ResourceSetType),
    getter("disabled", O::ElementDisabled), setter("disabled", O::ElementSetDisabled),
];
const META: &[Member] = &[
    getter("name", O::ElementName), setter("name", O::ElementSetName),
    getter("content", O::MetaContent), setter("content", O::MetaSetContent),
    getter("httpEquiv", O::MetaHttpEquiv), setter("httpEquiv", O::MetaSetHttpEquiv),
    getter("charset", O::MetaCharset), setter("charset", O::MetaSetCharset),
];
const CELL: &[Member] = &[
    getter("colSpan", O::CellColSpan), setter("colSpan", O::CellSetColSpan),
    getter("rowSpan", O::CellRowSpan), setter("rowSpan", O::CellSetRowSpan),
];
const OLIST: &[Member] = &[
    getter("start", O::OListStart), setter("start", O::OListSetStart),
    getter("reversed", O::OListReversed), setter("reversed", O::OListSetReversed),
];
const LI: &[Member] = &[getter("value", O::LIValue), setter("value", O::LISetValue)];
pub static INTERFACES: &[D] = &[
    D {
        interface: I::EventTarget,
        name: "EventTarget",
        parent: None,
        constructor: Some(O::EventTargetConstructor),
        members: EVENT_TARGET,
    },
    D { interface: I::AbortController, name: "AbortController", parent: None,
        constructor: Some(O::AbortControllerConstructor), members: ABORT_CONTROLLER },
    D { interface: I::AbortSignal, name: "AbortSignal", parent: Some(I::EventTarget),
        constructor: None, members: ABORT_SIGNAL },
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
    D { interface: I::ShadowRoot, name: "ShadowRoot", parent: Some(I::DocumentFragment),
        constructor: None, members: SHADOW_ROOT },
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
    D { interface: I::HTMLElement, name: "HTMLElement", parent: Some(I::Element), constructor: None, members: HTML_ELEMENT },
    D { interface: I::HTMLUnknownElement, name: "HTMLUnknownElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLHtmlElement, name: "HTMLHtmlElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLHeadElement, name: "HTMLHeadElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLTitleElement, name: "HTMLTitleElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLBaseElement, name: "HTMLBaseElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLLinkElement, name: "HTMLLinkElement", parent: Some(I::HTMLElement), constructor: None, members: LINK },
    D { interface: I::HTMLMetaElement, name: "HTMLMetaElement", parent: Some(I::HTMLElement), constructor: None, members: META },
    D { interface: I::HTMLStyleElement, name: "HTMLStyleElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLBodyElement, name: "HTMLBodyElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLHeadingElement, name: "HTMLHeadingElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLParagraphElement, name: "HTMLParagraphElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLHRElement, name: "HTMLHRElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLPreElement, name: "HTMLPreElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLQuoteElement, name: "HTMLQuoteElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLOListElement, name: "HTMLOListElement", parent: Some(I::HTMLElement), constructor: None, members: OLIST },
    D { interface: I::HTMLUListElement, name: "HTMLUListElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLLIElement, name: "HTMLLIElement", parent: Some(I::HTMLElement), constructor: None, members: LI },
    D { interface: I::HTMLDListElement, name: "HTMLDListElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLDivElement, name: "HTMLDivElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLAnchorElement, name: "HTMLAnchorElement", parent: Some(I::HTMLElement), constructor: None, members: HYPERLINK },
    D { interface: I::HTMLImageElement, name: "HTMLImageElement", parent: Some(I::HTMLElement), constructor: None, members: IMAGE },
    D { interface: I::HTMLBRElement, name: "HTMLBRElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLModElement, name: "HTMLModElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLTimeElement, name: "HTMLTimeElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLDataElement, name: "HTMLDataElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLTableElement, name: "HTMLTableElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLTableCaptionElement, name: "HTMLTableCaptionElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLTableSectionElement, name: "HTMLTableSectionElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLTableRowElement, name: "HTMLTableRowElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLTableCellElement, name: "HTMLTableCellElement", parent: Some(I::HTMLElement), constructor: None, members: CELL },
    D { interface: I::HTMLTableColElement, name: "HTMLTableColElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLFormElement, name: "HTMLFormElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLInputElement, name: "HTMLInputElement", parent: Some(I::HTMLElement), constructor: None, members: INPUT },
    D { interface: I::HTMLButtonElement, name: "HTMLButtonElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLSelectElement, name: "HTMLSelectElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLOptionElement, name: "HTMLOptionElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLOptGroupElement, name: "HTMLOptGroupElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLTextAreaElement, name: "HTMLTextAreaElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLLabelElement, name: "HTMLLabelElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLFieldSetElement, name: "HTMLFieldSetElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLLegendElement, name: "HTMLLegendElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLDataListElement, name: "HTMLDataListElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLOutputElement, name: "HTMLOutputElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLProgressElement, name: "HTMLProgressElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLMeterElement, name: "HTMLMeterElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLMediaElement, name: "HTMLMediaElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLVideoElement, name: "HTMLVideoElement", parent: Some(I::HTMLMediaElement), constructor: None, members: &[] },
    D { interface: I::HTMLAudioElement, name: "HTMLAudioElement", parent: Some(I::HTMLMediaElement), constructor: None, members: &[] },
    D { interface: I::HTMLSourceElement, name: "HTMLSourceElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLTrackElement, name: "HTMLTrackElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLCanvasElement, name: "HTMLCanvasElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLScriptElement, name: "HTMLScriptElement", parent: Some(I::HTMLElement), constructor: None, members: SCRIPT },
    D { interface: I::HTMLTemplateElement, name: "HTMLTemplateElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLSlotElement, name: "HTMLSlotElement", parent: Some(I::HTMLElement), constructor: None, members: SLOT },
    D { interface: I::HTMLIFrameElement, name: "HTMLIFrameElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLEmbedElement, name: "HTMLEmbedElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLObjectElement, name: "HTMLObjectElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLParamElement, name: "HTMLParamElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLDetailsElement, name: "HTMLDetailsElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLDialogElement, name: "HTMLDialogElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLMapElement, name: "HTMLMapElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLAreaElement, name: "HTMLAreaElement", parent: Some(I::HTMLElement), constructor: None, members: HYPERLINK },
    D { interface: I::HTMLPictureElement, name: "HTMLPictureElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLMenuElement, name: "HTMLMenuElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
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
        members: TEXT,
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
    D { interface: I::HTMLCollection, name: "HTMLCollection", parent: None, constructor: None, members: HTML_COLLECTION },
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
    D { interface: I::CustomElementRegistry, name: "CustomElementRegistry", parent: None,
        constructor: None, members: CUSTOM_ELEMENT_REGISTRY },
];

/// Primary HTML interface selected from a lower-case local name.
/// The native wrapper keeps its Node brand; the driver uses this lookup for
/// the prototype when it creates the canonical wrapper.
pub fn html_element_interface(local: &str) -> I {
    match local {
        "html" => I::HTMLHtmlElement,
        "head" => I::HTMLHeadElement,
        "title" => I::HTMLTitleElement,
        "base" => I::HTMLBaseElement,
        "link" => I::HTMLLinkElement,
        "meta" => I::HTMLMetaElement,
        "style" => I::HTMLStyleElement,
        "body" => I::HTMLBodyElement,
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => I::HTMLHeadingElement,
        "p" => I::HTMLParagraphElement,
        "hr" => I::HTMLHRElement,
        "pre" | "listing" | "xmp" => I::HTMLPreElement,
        "blockquote" | "q" => I::HTMLQuoteElement,
        "ol" => I::HTMLOListElement,
        "ul" => I::HTMLUListElement,
        "li" => I::HTMLLIElement,
        "dl" => I::HTMLDListElement,
        "div" => I::HTMLDivElement,
        "a" => I::HTMLAnchorElement,
        "img" => I::HTMLImageElement,
        "br" => I::HTMLBRElement,
        "ins" | "del" => I::HTMLModElement,
        "time" => I::HTMLTimeElement,
        "data" => I::HTMLDataElement,
        "table" => I::HTMLTableElement,
        "caption" => I::HTMLTableCaptionElement,
        "thead" | "tbody" | "tfoot" => I::HTMLTableSectionElement,
        "tr" => I::HTMLTableRowElement,
        "td" | "th" => I::HTMLTableCellElement,
        "col" | "colgroup" => I::HTMLTableColElement,
        "form" => I::HTMLFormElement,
        "input" => I::HTMLInputElement,
        "button" => I::HTMLButtonElement,
        "select" => I::HTMLSelectElement,
        "option" => I::HTMLOptionElement,
        "optgroup" => I::HTMLOptGroupElement,
        "textarea" => I::HTMLTextAreaElement,
        "label" => I::HTMLLabelElement,
        "fieldset" => I::HTMLFieldSetElement,
        "legend" => I::HTMLLegendElement,
        "datalist" => I::HTMLDataListElement,
        "output" => I::HTMLOutputElement,
        "progress" => I::HTMLProgressElement,
        "meter" => I::HTMLMeterElement,
        "video" => I::HTMLVideoElement,
        "audio" => I::HTMLAudioElement,
        "source" => I::HTMLSourceElement,
        "track" => I::HTMLTrackElement,
        "canvas" => I::HTMLCanvasElement,
        "script" => I::HTMLScriptElement,
        "template" => I::HTMLTemplateElement,
        "slot" => I::HTMLSlotElement,
        "iframe" => I::HTMLIFrameElement,
        "embed" => I::HTMLEmbedElement,
        "object" => I::HTMLObjectElement,
        "param" => I::HTMLParamElement,
        "details" => I::HTMLDetailsElement,
        "dialog" => I::HTMLDialogElement,
        "map" => I::HTMLMapElement,
        "area" => I::HTMLAreaElement,
        "picture" => I::HTMLPictureElement,
        "menu" => I::HTMLMenuElement,
        "article" | "aside" | "b" | "bdi" | "bdo" | "big" | "center" | "cite" | "code" | "dd" | "dfn" | "dt" | "em" | "figcaption" | "figure" | "footer" | "header" | "hgroup" | "i" | "kbd" | "main" | "mark" | "nav" | "nobr" | "noembed" | "noframes" | "noscript" | "plaintext" | "rb" | "rp" | "rt" | "rtc" | "ruby" | "s" | "samp" | "search" | "section" | "small" | "span" | "strike" | "strong" | "sub" | "sup" | "tt" | "u" | "var" | "wbr" | "address" | "abbr" | "acronym" | "basefont" => I::HTMLElement,
        name if name.contains('-') => I::HTMLElement,
        _ => I::HTMLUnknownElement,
    }
}

/// Add these descriptors without deleting unrelated existing browser APIs.
/// Constructors require `new`; illegal constructors still expose prototypes.
/// Runtime defines writable/configurable methods, enumerable configurable
/// accessors, constructor/prototype links, and the proper function lengths.
pub fn install<H: BindingHost>(host: &mut H) -> Result<InstallReport, H::Error> {
    let mut report = InstallReport::default();
    for definition in INTERFACES {
        host.install_interface(definition)?;
        if host.install_interface_class_string(definition.interface, definition.name)?
            == OptionalHookInstall::NotSupported {
            report.not_installed.push(OptionalCapability::InterfaceClassString(definition.interface));
        }
    }
    if host.install_dom_exception_error_prototype()? == OptionalHookInstall::NotSupported {
        report.not_installed.push(OptionalCapability::DomExceptionErrorPrototype);
    }
    if host.install_custom_elements_global()? == OptionalHookInstall::NotSupported {
        report.not_installed.push(OptionalCapability::CustomElementsGlobal);
    }
    host.install_static_method(I::AbortSignal, "abort", O::AbortSignalAbort, 0)?;
    host.install_static_method(I::AbortSignal, "any", O::AbortSignalAny, 1)?;
    host.install_static_method(I::AbortSignal, "timeout", O::AbortSignalTimeout, 1)?;
    host.define_legacy_platform_object(InterfaceBrand::COLLECTION, LegacyPlatformHooks {
        supported_index_count: O::HTMLCollectionSupportedIndexCount,
        supported_names: Some(O::HTMLCollectionSupportedNames),
        indexed_getter: O::HTMLCollectionIndexedGetter,
        indexed_setter: None,
        named_getter: Some(O::HTMLCollectionNamedGetter),
        named_setter: None,
        named_deleter: None,
        override_builtins: false,
        unenumerable_names: true,
    })?;
    for brand in [InterfaceBrand::NODE_LIST, InterfaceBrand::STATIC_NODE_LIST, InterfaceBrand::FILTERED_NODE_LIST] {
        host.define_legacy_platform_object(brand, LegacyPlatformHooks {
            supported_index_count: O::NodeListSupportedIndexCount, supported_names: None,
            indexed_getter: O::NodeListIndexedGetter, indexed_setter: None, named_getter: None,
            named_setter: None, named_deleter: None, override_builtins: false,
            unenumerable_names: false,
        })?;
    }
    host.define_legacy_platform_object(InterfaceBrand::DOM_TOKEN_LIST, LegacyPlatformHooks {
        supported_index_count: O::TokenListSupportedIndexCount, supported_names: None,
        indexed_getter: O::TokenListIndexedGetter, indexed_setter: None, named_getter: None,
        named_setter: None, named_deleter: None, override_builtins: false,
        unenumerable_names: false,
    })?;
    host.define_iterable(I::NodeList, IterableKind::Values)?;
    host.define_iterable(I::DOMTokenList, IterableKind::Values)?;
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
    Ok(report)
}
