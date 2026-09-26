use amazing_dom::legacy_document::LegacyColor as LC;
use crate::host::{
    BindingHost, FormAttribute as F, Interface as I, InterfaceBrand, InterfaceDefinition as D, IterableKind, LegacyPlatformHooks, Member, MemberKind as K, Operation as O, OptionalHookInstall,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionalCapability {
    InterfaceClassString(I),
    DomExceptionErrorPrototype,
    CustomElementsGlobal,
    PairIterable(I),
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
const fn form_get(name: &'static str, attr: F) -> Member {
    getter(name, O::FormAttributeGet(attr))
}
const fn form_set(name: &'static str, attr: F) -> Member {
    setter(name, O::FormAttributeSet(attr))
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
    method("lookupNamespaceURI", O::NodeLookupNamespaceURI, 1),
    method("isDefaultNamespace", O::NodeIsDefaultNamespace, 1),
    method("normalize", O::NodeNormalize, 0),
    method("isSameNode", O::NodeIsSameNode, 1),
    method("isEqualNode", O::NodeIsEqualNode, 1),
    method("compareDocumentPosition", O::NodeCompareDocumentPosition, 1),
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
const UI_EVENT: &[Member] = &[
    getter("view", O::EventExtra("view")),
    getter("detail", O::EventExtra("detail")),
    getter("which", O::EventExtra("which")),
];
const FOCUS_EVENT: &[Member] = &[getter("relatedTarget", O::EventExtra("relatedTarget"))];
const MOUSE_EVENT: &[Member] = &[
    method("getModifierState", O::MouseGetModifierState, 1),
    getter("ctrlKey", O::EventExtra("ctrlKey")),
    getter("shiftKey", O::EventExtra("shiftKey")),
    getter("altKey", O::EventExtra("altKey")),
    getter("metaKey", O::EventExtra("metaKey")),
    getter("screenX", O::EventExtra("screenX")),
    getter("screenY", O::EventExtra("screenY")),
    getter("clientX", O::EventExtra("clientX")),
    getter("clientY", O::EventExtra("clientY")),
    getter("button", O::EventExtra("button")),
    getter("buttons", O::EventExtra("buttons")),
    getter("relatedTarget", O::EventExtra("relatedTarget")),
];
const WHEEL_EVENT: &[Member] = &[
    getter("deltaX", O::EventExtra("deltaX")),
    getter("deltaY", O::EventExtra("deltaY")),
    getter("deltaZ", O::EventExtra("deltaZ")),
    getter("deltaMode", O::EventExtra("deltaMode")),
    getter("momentum", O::EventExtra("momentum")),
];
const KEYBOARD_EVENT: &[Member] = &[
    method("getModifierState", O::KeyboardGetModifierState, 1),
    getter("ctrlKey", O::EventExtra("ctrlKey")),
    getter("shiftKey", O::EventExtra("shiftKey")),
    getter("altKey", O::EventExtra("altKey")),
    getter("metaKey", O::EventExtra("metaKey")),
    getter("key", O::EventExtra("key")),
    getter("code", O::EventExtra("code")),
    getter("location", O::EventExtra("location")),
    getter("repeat", O::EventExtra("repeat")),
    getter("isComposing", O::EventExtra("isComposing")),
    getter("charCode", O::EventExtra("charCode")),
    getter("keyCode", O::EventExtra("keyCode")),
];
const MESSAGE_EVENT: &[Member] = &[
    getter("data", O::MessageEventGet("data")),
    getter("origin", O::MessageEventGet("origin")),
    getter("lastEventId", O::MessageEventGet("lastEventId")),
    getter("source", O::MessageEventGet("source")),
    getter("ports", O::MessageEventGet("ports")),
    method("initMessageEvent", O::MessageEventInit, 1),
];
const COMPOSITION_EVENT: &[Member] = &[getter("data", O::EventExtra("data"))];
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
    method("moveBefore", O::MoveBefore, 2),
    method("getElementById", O::FragmentGetElementById, 1),
    method("querySelector", O::ParentQuerySelector, 1),
    method("querySelectorAll", O::ParentQuerySelectorAll, 1),
    getter("children", O::FragmentChildren),
    getter("firstElementChild", O::FirstElementChild),
    getter("lastElementChild", O::LastElementChild),
    getter("childElementCount", O::ChildElementCount),
    method("append", O::Append, 0),
    method("prepend", O::Prepend, 0),
    method("replaceChildren", O::ReplaceChildren, 0),
];
const CHARACTER_DATA: &[Member] = &[
    getter("previousElementSibling", O::PreviousElementSibling),
    getter("nextElementSibling", O::NextElementSibling),
    getter("data", O::Data),
    setter("data", O::SetData),
    getter("length", O::Length),
    method("substringData", O::CharacterDataSubstring, 2),
    method("appendData", O::CharacterDataAppend, 1),
    method("insertData", O::CharacterDataInsert, 2),
    method("deleteData", O::CharacterDataDelete, 2),
    method("replaceData", O::CharacterDataReplace, 3),
    method("before", O::Before, 0),
    method("after", O::After, 0),
    method("replaceWith", O::ReplaceWith, 0),
    method("remove", O::Remove, 0),
];
const DOCUMENT: &[Member] = &[
    getter("fgColor", O::DocumentLegacyColorGet(LC::Text)), setter("fgColor", O::DocumentLegacyColorSet(LC::Text)),
    getter("bgColor", O::DocumentLegacyColorGet(LC::Background)), setter("bgColor", O::DocumentLegacyColorSet(LC::Background)),
    getter("linkColor", O::DocumentLegacyColorGet(LC::Link)), setter("linkColor", O::DocumentLegacyColorSet(LC::Link)),
    getter("vlinkColor", O::DocumentLegacyColorGet(LC::VisitedLink)), setter("vlinkColor", O::DocumentLegacyColorSet(LC::VisitedLink)),
    getter("alinkColor", O::DocumentLegacyColorGet(LC::ActiveLink)), setter("alinkColor", O::DocumentLegacyColorSet(LC::ActiveLink)),
    method("clear", O::DocumentLegacyNoop, 0),
    method("captureEvents", O::DocumentLegacyNoop, 0),
    method("releaseEvents", O::DocumentLegacyNoop, 0),
    method("moveBefore", O::MoveBefore, 2),
    getter("location", O::DocumentLocation),
    getter("fonts", O::DocumentFonts),
    getter("title", O::DocumentTitle), setter("title", O::DocumentSetTitle),
    getter("head", O::DocumentHead),
    getter("body", O::DocumentBody), setter("body", O::DocumentSetBody),
    getter("documentElement", O::DocumentElement),
    getter("doctype", O::DocumentDoctype),
    getter("readyState", O::DocumentReadyState),
    getter("defaultView", O::DocumentDefaultView),
    getter("URL", O::DocumentURL), getter("documentURI", O::DocumentURI),
    getter("characterSet", O::DocumentCharacterSet),
    getter("charset", O::DocumentCharacterSet), getter("inputEncoding", O::DocumentCharacterSet),
    getter("contentType", O::DocumentContentType),
    getter("compatMode", O::DocumentCompatMode), getter("referrer", O::DocumentReferrer),
    method("getElementById", O::DocumentGetElementById, 1),
    method("getSelection", O::GetSelection, 0),
    method("getElementsByName", O::DocumentGetElementsByName, 1),
    method("adoptNode", O::DocumentAdoptNode, 1),
    method("importNode", O::DocumentImportNode, 1),
    getter("forms", O::DocumentForms), getter("images", O::DocumentImages),
    getter("embeds", O::DocumentEmbeds), getter("plugins", O::DocumentEmbeds),
    getter("anchors", O::DocumentAnchors), getter("applets", O::DocumentApplets),
    getter("links", O::DocumentLinks), getter("scripts", O::DocumentScripts),
    getter("activeElement", O::DocumentActiveElement),
    method("hasFocus", O::DocumentHasFocus, 0),
    method("querySelector", O::ParentQuerySelector, 1),
    method("querySelectorAll", O::ParentQuerySelectorAll, 1),
    getter("children", O::DocumentChildren),
    getter("firstElementChild", O::FirstElementChild),
    getter("lastElementChild", O::LastElementChild),
    getter("childElementCount", O::ChildElementCount),
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
const URL: &[Member] = &[
    getter("href", O::URLHref), setter("href", O::URLSetHref),
    getter("origin", O::URLOrigin),
    getter("protocol", O::URLProtocol), setter("protocol", O::URLSetProtocol),
    getter("username", O::URLUsername), setter("username", O::URLSetUsername),
    getter("password", O::URLPassword), setter("password", O::URLSetPassword),
    getter("host", O::URLHost), setter("host", O::URLSetHost),
    getter("hostname", O::URLHostname), setter("hostname", O::URLSetHostname),
    getter("port", O::URLPort), setter("port", O::URLSetPort),
    getter("pathname", O::URLPathname), setter("pathname", O::URLSetPathname),
    getter("search", O::URLSearch), setter("search", O::URLSetSearch),
    getter("searchParams", O::URLSearchParams),
    getter("hash", O::URLHash), setter("hash", O::URLSetHash),
    method("toString", O::URLToString, 0), method("toJSON", O::URLToJSON, 0),
];
const URL_SEARCH_PARAMS: &[Member] = &[
    getter("size", O::URLSearchParamsSize),
    method("append", O::URLSearchParamsAppend, 2),
    method("delete", O::URLSearchParamsDelete, 1),
    method("get", O::URLSearchParamsGet, 1),
    method("getAll", O::URLSearchParamsGetAll, 1),
    method("has", O::URLSearchParamsHas, 1),
    method("set", O::URLSearchParamsSet, 2),
    method("sort", O::URLSearchParamsSort, 0),
    method("toString", O::URLSearchParamsToString, 0),
    method("forEach", O::URLSearchParamsForEach, 1),
];
const BLOB: &[Member] = &[
    getter("size", O::BlobSize), getter("type", O::BlobType),
    method("slice", O::BlobSlice, 0), method("text", O::BlobText, 0),
    method("arrayBuffer", O::BlobArrayBuffer, 0), method("bytes", O::BlobBytes, 0),
    method("stream", O::BlobStream, 0),
];
const FILE: &[Member] = &[
    getter("name", O::FileName), getter("lastModified", O::FileLastModified),
];
const FORM_DATA: &[Member] = &[
    method("append", O::FormDataAppend, 2), method("delete", O::FormDataDelete, 1),
    method("get", O::FormDataGet, 1), method("getAll", O::FormDataGetAll, 1),
    method("has", O::FormDataHas, 1), method("set", O::FormDataSet, 2),
    method("forEach", O::FormDataForEach, 1),
];
const FONT_FACE_SET: &[Member] = &[
    getter("status", O::FontFaceSetStatus), getter("ready", O::FontFaceSetReady),
];
const DOM_PARSER: &[Member] = &[
    method("parseFromString", O::DOMParserParseFromString, 2),
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
const LOCATION: &[Member] = &[
    getter("href", O::LocationHref), setter("href", O::LocationSetHref),
    getter("origin", O::LocationOrigin),
    getter("protocol", O::LocationProtocol), setter("protocol", O::LocationSetProtocol),
    getter("host", O::LocationHost), setter("host", O::LocationSetHost),
    getter("hostname", O::LocationHostname), setter("hostname", O::LocationSetHostname),
    getter("port", O::LocationPort), setter("port", O::LocationSetPort),
    getter("pathname", O::LocationPathname), setter("pathname", O::LocationSetPathname),
    getter("search", O::LocationSearch), setter("search", O::LocationSetSearch),
    getter("hash", O::LocationHash), setter("hash", O::LocationSetHash),
    method("assign", O::LocationAssign, 1), method("replace", O::LocationReplace, 1),
    method("reload", O::LocationReload, 0),
    getter("ancestorOrigins", O::LocationAncestorOrigins),
    method("toString", O::LocationToString, 0),
];
const DOM_STRING_LIST: &[Member] = &[
    getter("length", O::DOMStringListLength),
    method("item", O::DOMStringListItem, 1),
    method("contains", O::DOMStringListContains, 1),
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
const SELECTION: &[Member] = &[
    getter("anchorNode", O::SelectionAnchorNode),
    getter("anchorOffset", O::SelectionAnchorOffset),
    getter("focusNode", O::SelectionFocusNode),
    getter("focusOffset", O::SelectionFocusOffset),
    getter("isCollapsed", O::SelectionIsCollapsed),
    getter("rangeCount", O::SelectionRangeCount),
    method("getRangeAt", O::SelectionGetRangeAt, 1),
    method("addRange", O::SelectionAddRange, 1),
    method("removeRange", O::SelectionRemoveRange, 1),
    method("removeAllRanges", O::SelectionRemoveAllRanges, 0),
    method("empty", O::SelectionRemoveAllRanges, 0),
    method("collapse", O::SelectionCollapse, 1),
    method("selectAllChildren", O::SelectionSelectAllChildren, 1),
    method("toString", O::SelectionToString, 0),
];
const ELEMENT: &[Member] = &[
    method("moveBefore", O::MoveBefore, 2),
    getter("namespaceURI", O::ElementNamespaceURI),
    getter("prefix", O::ElementPrefix),
    getter("localName", O::ElementLocalName),
    getter("tagName", O::ElementTagName),
    method("attachShadow", O::ElementAttachShadow, 1),
    getter("shadowRoot", O::ElementShadowRoot),
    getter("assignedSlot", O::ElementAssignedSlot),
    getter("slot", O::ElementSlot), setter("slot", O::ElementSetSlot),
    getter("innerHTML", O::ElementInnerHTML), setter("innerHTML", O::ElementSetInnerHTML),
    method("getHTML", O::ElementGetHTML, 0),
    getter("outerHTML", O::ElementOuterHTML), setter("outerHTML", O::ElementSetOuterHTML),
    method("insertAdjacentHTML", O::ElementInsertAdjacentHTML, 2),
    method("insertAdjacentElement", O::ElementInsertAdjacentElement, 2),
    method("insertAdjacentText", O::ElementInsertAdjacentText, 2),
    method("querySelector", O::ParentQuerySelector, 1),
    method("querySelectorAll", O::ParentQuerySelectorAll, 1),
    method("matches", O::ElementMatches, 1),
    method("closest", O::ElementClosest, 1),
    method("webkitMatchesSelector", O::ElementWebkitMatchesSelector, 1),
    getter("id", O::ElementId),
    setter("id", O::ElementSetId),
    getter("children", O::ElementChildren),
    getter("firstElementChild", O::FirstElementChild),
    getter("lastElementChild", O::LastElementChild),
    getter("childElementCount", O::ChildElementCount),
    getter("previousElementSibling", O::PreviousElementSibling),
    getter("nextElementSibling", O::NextElementSibling),
    method("getElementsByTagName", O::ElementGetElementsByTagName, 1),
    method("getElementsByTagNameNS", O::ElementGetElementsByTagNameNS, 2),
    method("getElementsByClassName", O::ElementGetElementsByClassName, 1),
    getter("classList", O::ElementClassList),
    setter("classList", O::ElementSetClassName),
    getter("className", O::ElementClassName),
    setter("className", O::ElementSetClassName),
    getter("attributes", O::ElementAttributes),
    method("hasAttributes", O::ElementHasAttributes, 0),
    method("getAttributeNames", O::ElementGetAttributeNames, 0),
    method("hasAttribute", O::ElementHasAttribute, 1),
    method("hasAttributeNS", O::ElementHasAttributeNS, 2),
    method("toggleAttribute", O::ElementToggleAttribute, 1),
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
const DOCUMENT_TYPE: &[Member] = &[
    getter("name", O::DocumentTypeName),
    getter("publicId", O::DocumentTypePublicId),
    getter("systemId", O::DocumentTypeSystemId),
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
    method("getHTML", O::ElementGetHTML, 0),
];
const SLOT: &[Member] = &[
    getter("name", O::SlotName), setter("name", O::SlotSetName),
    method("assign", O::SlotAssign, 0),
    method("assignedNodes", O::SlotAssignedNodes, 0),
    method("assignedElements", O::SlotAssignedElements, 0),
];
const TEXT: &[Member] = &[
    getter("assignedSlot", O::TextAssignedSlot),
    getter("wholeText", O::TextWholeText),
    method("splitText", O::TextSplit, 1),
];
const HTML_BODY: &[Member] = &[
    getter("text", O::BodyLegacyColorGet(LC::Text)), setter("text", O::BodyLegacyColorSet(LC::Text)),
    getter("bgColor", O::BodyLegacyColorGet(LC::Background)), setter("bgColor", O::BodyLegacyColorSet(LC::Background)),
    getter("link", O::BodyLegacyColorGet(LC::Link)), setter("link", O::BodyLegacyColorSet(LC::Link)),
    getter("vLink", O::BodyLegacyColorGet(LC::VisitedLink)), setter("vLink", O::BodyLegacyColorSet(LC::VisitedLink)),
    getter("aLink", O::BodyLegacyColorGet(LC::ActiveLink)), setter("aLink", O::BodyLegacyColorSet(LC::ActiveLink)),
];
const HTML_ELEMENT: &[Member] = &[
    getter("dataset", O::ElementDataset),
    method("focus", O::ElementFocus, 0),
    method("blur", O::ElementBlur, 0),
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
    getter("relList", O::ElementRelList), setter("relList", O::ElementSetRelList),
    getter("download", O::ElementDownload), setter("download", O::ElementSetDownload),
];
const IMAGE: &[Member] = &[
    getter("src", O::ElementSrc), setter("src", O::ElementSetSrc),
    getter("srcset", O::ImageSrcset), setter("srcset", O::ImageSetSrcset),
    getter("sizes", O::ImageSizes), setter("sizes", O::ImageSetSizes),
    getter("currentSrc", O::ImageCurrentSrc),
    getter("decoding", O::ImageDecoding), setter("decoding", O::ImageSetDecoding),
    getter("complete", O::ImageComplete),
    getter("naturalWidth", O::ImageNaturalWidth),
    getter("naturalHeight", O::ImageNaturalHeight),
    method("decode", O::ImageDecode, 0),
    getter("alt", O::ImageAlt), setter("alt", O::ImageSetAlt),
    getter("width", O::ImageWidth), setter("width", O::ImageSetWidth),
    getter("height", O::ImageHeight), setter("height", O::ImageSetHeight),
    getter("loading", O::ImageLoading), setter("loading", O::ImageSetLoading),
];
const SOURCE: &[Member] = &[
    getter("type", O::SourceType), setter("type", O::SourceSetType),
    getter("media", O::SourceMedia), setter("media", O::SourceSetMedia),
    getter("srcset", O::SourceSrcset), setter("srcset", O::SourceSetSrcset),
    getter("sizes", O::SourceSizes), setter("sizes", O::SourceSetSizes),
];
const INPUT: &[Member] = &[
    form_get("accept", F::Accept), form_set("accept", F::Accept),
    form_get("alpha", F::Alpha), form_set("alpha", F::Alpha),
    form_get("alt", F::Alt), form_set("alt", F::Alt),
    form_get("autocomplete", F::Autocomplete), form_set("autocomplete", F::Autocomplete),
    form_get("dirName", F::DirName), form_set("dirName", F::DirName),
    form_get("formAction", F::FormAction), form_set("formAction", F::FormAction),
    form_get("formEnctype", F::FormEnctype), form_set("formEnctype", F::FormEnctype),
    form_get("formMethod", F::FormMethod), form_set("formMethod", F::FormMethod),
    form_get("formNoValidate", F::FormNoValidate), form_set("formNoValidate", F::FormNoValidate),
    form_get("formTarget", F::FormTarget), form_set("formTarget", F::FormTarget),
    form_get("height", F::Height), form_set("height", F::Height),
    form_get("max", F::Max), form_set("max", F::Max),
    form_get("maxLength", F::MaxLength), form_set("maxLength", F::MaxLength),
    form_get("min", F::Min), form_set("min", F::Min),
    form_get("minLength", F::MinLength), form_set("minLength", F::MinLength),
    form_get("multiple", F::Multiple), form_set("multiple", F::Multiple),
    form_get("pattern", F::Pattern), form_set("pattern", F::Pattern),
    form_get("readOnly", F::ReadOnly), form_set("readOnly", F::ReadOnly),
    form_get("required", F::Required), form_set("required", F::Required),
    form_get("size", F::Size), form_set("size", F::Size),
    form_get("src", F::Src), form_set("src", F::Src),
    form_get("step", F::Step), form_set("step", F::Step),
    form_get("width", F::Width), form_set("width", F::Width),
    getter("type", O::InputType), setter("type", O::InputSetType),
    getter("name", O::ElementName), setter("name", O::ElementSetName),
    getter("disabled", O::ElementDisabled), setter("disabled", O::ElementSetDisabled),
    getter("placeholder", O::InputPlaceholder), setter("placeholder", O::InputSetPlaceholder),
    getter("form", O::FormControlForm),
    getter("value", O::InputValue), setter("value", O::InputSetValue),
    getter("defaultValue", O::InputDefaultValue), setter("defaultValue", O::InputSetDefaultValue),
    getter("checked", O::InputChecked), setter("checked", O::InputSetChecked),
    getter("defaultChecked", O::InputDefaultChecked), setter("defaultChecked", O::InputSetDefaultChecked),
    getter("valueAsNumber", O::InputValueAsNumber), setter("valueAsNumber", O::InputSetValueAsNumber),
    getter("valueAsDate", O::InputValueAsDate), setter("valueAsDate", O::InputSetValueAsDate),
    getter("selectionStart", O::InputSelectionStart), setter("selectionStart", O::InputSetSelectionStart),
    getter("selectionEnd", O::InputSelectionEnd), setter("selectionEnd", O::InputSetSelectionEnd),
    getter("selectionDirection", O::InputSelectionDirection), setter("selectionDirection", O::InputSetSelectionDirection),
    method("select", O::InputSelect, 0), method("setSelectionRange", O::InputSetSelectionRange, 2),
    getter("willValidate", O::ControlWillValidate), getter("validity", O::ControlValidity),
    getter("validationMessage", O::ControlValidationMessage),
    method("checkValidity", O::ControlCheckValidity, 0), method("reportValidity", O::ControlReportValidity, 0),
    method("setCustomValidity", O::ControlSetCustomValidity, 1),
];
const FORM: &[Member] = &[
    form_get("acceptCharset", F::AcceptCharset), form_set("acceptCharset", F::AcceptCharset),
    form_get("action", F::Action), form_set("action", F::Action),
    form_get("autocomplete", F::Autocomplete), form_set("autocomplete", F::Autocomplete),
    form_get("enctype", F::Enctype), form_set("enctype", F::Enctype),
    form_get("encoding", F::Encoding), form_set("encoding", F::Encoding),
    form_get("method", F::Method), form_set("method", F::Method),
    form_get("name", F::Name), form_set("name", F::Name),
    form_get("noValidate", F::NoValidate), form_set("noValidate", F::NoValidate),
    form_get("target", F::Target), form_set("target", F::Target),
    form_get("rel", F::Rel), form_set("rel", F::Rel),
    form_get("relList", F::RelList), form_set("relList", F::RelList),
    getter("elements", O::FormElements), getter("length", O::FormLength),
    method("reset", O::FormReset, 0), method("checkValidity", O::FormCheckValidity, 0),
    method("reportValidity", O::FormReportValidity, 0),
];
const FORM_CONTROL: &[Member] = &[
    form_get("disabled", F::Disabled), form_set("disabled", F::Disabled),
    getter("form", O::FormControlForm), getter("name", O::FormControlName), setter("name", O::FormControlSetName),
    getter("willValidate", O::ControlWillValidate), getter("validity", O::ControlValidity),
    getter("validationMessage", O::ControlValidationMessage),
    method("checkValidity", O::ControlCheckValidity, 0), method("reportValidity", O::ControlReportValidity, 0),
    method("setCustomValidity", O::ControlSetCustomValidity, 1),
];
const OUTPUT: &[Member] = &[
    getter("htmlFor", O::OutputHtmlFor), setter("htmlFor", O::OutputSetHtmlFor),
    getter("form", O::FormControlForm), getter("name", O::FormControlName), setter("name", O::FormControlSetName),
    getter("willValidate", O::ControlWillValidate), getter("validity", O::ControlValidity),
    getter("validationMessage", O::ControlValidationMessage),
    method("checkValidity", O::ControlCheckValidity, 0), method("reportValidity", O::ControlReportValidity, 0),
    method("setCustomValidity", O::ControlSetCustomValidity, 1),
];
const IFRAME: &[Member] = &[
    getter("sandbox", O::IFrameSandbox), setter("sandbox", O::IFrameSetSandbox),
];
const BUTTON: &[Member] = &[
    form_get("command", F::Command), form_set("command", F::Command),
    form_get("disabled", F::Disabled), form_set("disabled", F::Disabled),
    form_get("formAction", F::FormAction), form_set("formAction", F::FormAction),
    form_get("formEnctype", F::FormEnctype), form_set("formEnctype", F::FormEnctype),
    form_get("formMethod", F::FormMethod), form_set("formMethod", F::FormMethod),
    form_get("formNoValidate", F::FormNoValidate), form_set("formNoValidate", F::FormNoValidate),
    form_get("formTarget", F::FormTarget), form_set("formTarget", F::FormTarget),
    form_get("value", F::Value), form_set("value", F::Value),
    getter("type", O::ButtonType), setter("type", O::ButtonSetType),
    getter("form", O::FormControlForm), getter("name", O::FormControlName), setter("name", O::FormControlSetName),
    getter("willValidate", O::ControlWillValidate), getter("validity", O::ControlValidity),
    getter("validationMessage", O::ControlValidationMessage),
    method("checkValidity", O::ControlCheckValidity, 0), method("reportValidity", O::ControlReportValidity, 0),
    method("setCustomValidity", O::ControlSetCustomValidity, 1),
];
const TEXTAREA: &[Member] = &[
    form_get("autocomplete", F::Autocomplete), form_set("autocomplete", F::Autocomplete),
    form_get("cols", F::Cols), form_set("cols", F::Cols),
    form_get("dirName", F::DirName), form_set("dirName", F::DirName),
    form_get("disabled", F::Disabled), form_set("disabled", F::Disabled),
    form_get("maxLength", F::MaxLength), form_set("maxLength", F::MaxLength),
    form_get("minLength", F::MinLength), form_set("minLength", F::MinLength),
    form_get("name", F::Name), form_set("name", F::Name),
    form_get("placeholder", F::Placeholder), form_set("placeholder", F::Placeholder),
    form_get("readOnly", F::ReadOnly), form_set("readOnly", F::ReadOnly),
    form_get("required", F::Required), form_set("required", F::Required),
    form_get("rows", F::Rows), form_set("rows", F::Rows),
    form_get("wrap", F::Wrap), form_set("wrap", F::Wrap),
    getter("form", O::FormControlForm), getter("value", O::TextAreaValue), setter("value", O::TextAreaSetValue),
    getter("defaultValue", O::TextAreaDefaultValue), setter("defaultValue", O::TextAreaSetDefaultValue),
    getter("selectionStart", O::InputSelectionStart), setter("selectionStart", O::InputSetSelectionStart),
    getter("selectionEnd", O::InputSelectionEnd), setter("selectionEnd", O::InputSetSelectionEnd),
    getter("selectionDirection", O::InputSelectionDirection), setter("selectionDirection", O::InputSetSelectionDirection),
    method("select", O::InputSelect, 0), method("setSelectionRange", O::InputSetSelectionRange, 2),
    getter("willValidate", O::ControlWillValidate), getter("validity", O::ControlValidity),
    getter("validationMessage", O::ControlValidationMessage),
    method("checkValidity", O::ControlCheckValidity, 0), method("reportValidity", O::ControlReportValidity, 0),
    method("setCustomValidity", O::ControlSetCustomValidity, 1),
];
const SELECT: &[Member] = &[
    form_get("autocomplete", F::Autocomplete), form_set("autocomplete", F::Autocomplete),
    form_get("disabled", F::Disabled), form_set("disabled", F::Disabled),
    form_get("name", F::Name), form_set("name", F::Name),
    form_get("required", F::Required), form_set("required", F::Required),
    form_get("size", F::Size), form_set("size", F::Size),
    getter("form", O::FormControlForm), getter("value", O::SelectValue), setter("value", O::SelectSetValue),
    getter("selectedIndex", O::SelectSelectedIndex), setter("selectedIndex", O::SelectSetSelectedIndex),
    getter("options", O::SelectOptions), getter("length", O::SelectLength),
    getter("multiple", O::SelectMultiple), setter("multiple", O::SelectSetMultiple),
    method("add", O::SelectAdd, 1), method("remove", O::SelectRemove, 0),
    getter("willValidate", O::ControlWillValidate), getter("validity", O::ControlValidity),
    getter("validationMessage", O::ControlValidationMessage),
    method("checkValidity", O::ControlCheckValidity, 0), method("reportValidity", O::ControlReportValidity, 0),
    method("setCustomValidity", O::ControlSetCustomValidity, 1),
];
const OPTION: &[Member] = &[
    form_get("disabled", F::Disabled), form_set("disabled", F::Disabled),
    form_get("label", F::Label), form_set("label", F::Label),
    getter("value", O::OptionValue), setter("value", O::OptionSetValue),
    getter("selected", O::OptionSelected), setter("selected", O::OptionSetSelected),
    getter("defaultSelected", O::OptionDefaultSelected), setter("defaultSelected", O::OptionSetDefaultSelected),
    getter("text", O::OptionText), setter("text", O::OptionSetText),
];
const OPTGROUP: &[Member] = &[
    form_get("disabled", F::Disabled), form_set("disabled", F::Disabled),
    form_get("label", F::Label), form_set("label", F::Label),
];
const LABEL: &[Member] = &[
    form_get("htmlFor", F::HtmlFor), form_set("htmlFor", F::HtmlFor),
];
const VALIDITY: &[Member] = &[
    getter("valueMissing", O::ValidityValueMissing), getter("typeMismatch", O::ValidityTypeMismatch),
    getter("patternMismatch", O::ValidityPatternMismatch), getter("tooLong", O::ValidityTooLong),
    getter("tooShort", O::ValidityTooShort), getter("rangeUnderflow", O::ValidityRangeUnderflow),
    getter("rangeOverflow", O::ValidityRangeOverflow), getter("stepMismatch", O::ValidityStepMismatch),
    getter("badInput", O::ValidityBadInput), getter("customError", O::ValidityCustomError),
    getter("valid", O::ValidityValid),
];
const FORM_COLLECTION: &[Member] = &[
    getter("length", O::FormCollectionLength), method("item", O::FormCollectionItem, 1),
    method("namedItem", O::FormCollectionNamedItem, 1),
];
const OPTIONS_COLLECTION: &[Member] = &[
    getter("length", O::OptionsLength), setter("length", O::OptionsSetLength),
    getter("selectedIndex", O::OptionsSelectedIndex), setter("selectedIndex", O::OptionsSetSelectedIndex),
    method("item", O::OptionsItem, 1),
    method("namedItem", O::OptionsNamedItem, 1), method("add", O::SelectAdd, 1),
    method("remove", O::SelectRemove, 1),
];
const RADIO_LIST: &[Member] = &[
    getter("length", O::RadioNodeListLength), method("item", O::RadioNodeListItem, 1),
    getter("value", O::RadioNodeListValue), setter("value", O::RadioNodeListSetValue),
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
    getter("relList", O::ElementRelList), setter("relList", O::ElementSetRelList),
    getter("sizes", O::LinkSizes), setter("sizes", O::LinkSetSizes),
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
    D { interface: I::Location, name: "Location", parent: None,
        constructor: None, members: LOCATION },
    D { interface: I::DOMStringList, name: "DOMStringList", parent: None,
        constructor: None, members: DOM_STRING_LIST },
    D { interface: I::DOMParser, name: "DOMParser", parent: None,
        constructor: Some(O::DOMParserConstructor), members: DOM_PARSER },
    D { interface: I::URL, name: "URL", parent: None, constructor: Some(O::URLConstructor), members: URL },
    D { interface: I::URLSearchParams, name: "URLSearchParams", parent: None,
        constructor: Some(O::URLSearchParamsConstructor), members: URL_SEARCH_PARAMS },
    D { interface: I::Blob, name: "Blob", parent: None, constructor: Some(O::BlobConstructor), members: BLOB },
    D { interface: I::File, name: "File", parent: Some(I::Blob), constructor: Some(O::FileConstructor), members: FILE },
    D { interface: I::FormData, name: "FormData", parent: None, constructor: Some(O::FormDataConstructor), members: FORM_DATA },
    D {
        interface: I::EventTarget,
        name: "EventTarget",
        parent: None,
        constructor: Some(O::EventTargetConstructor),
        members: EVENT_TARGET,
    },
    D { interface: I::FontFaceSet, name: "FontFaceSet", parent: Some(I::EventTarget),
        constructor: None, members: FONT_FACE_SET },
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
    D { interface: I::XMLDocument, name: "XMLDocument", parent: Some(I::Document),
        constructor: None, members: &[] },
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
        members: DOCUMENT_TYPE,
    },
    D {
        interface: I::Element,
        name: "Element",
        parent: Some(I::Node),
        constructor: None,
        members: ELEMENT,
    },
    D { interface: I::HTMLElement, name: "HTMLElement", parent: Some(I::Element), constructor: None, members: HTML_ELEMENT },
    D { interface: I::SVGElement, name: "SVGElement", parent: Some(I::Element), constructor: None,
        members: &[getter("dataset", O::ElementDataset), method("focus", O::ElementFocus, 0), method("blur", O::ElementBlur, 0)] },
    D { interface: I::MathMLElement, name: "MathMLElement", parent: Some(I::Element), constructor: None,
        members: &[getter("dataset", O::ElementDataset), method("focus", O::ElementFocus, 0), method("blur", O::ElementBlur, 0)] },
    D { interface: I::HTMLDirectoryElement, name: "HTMLDirectoryElement", parent: Some(I::HTMLElement), constructor: None, members: &[
        getter("compact", O::DirectoryCompact), setter("compact", O::DirectoryCompactSet)
    ] },
    D { interface: I::HTMLFontElement, name: "HTMLFontElement", parent: Some(I::HTMLElement), constructor: None, members: &[
        getter("color", O::FontColor), setter("color", O::FontColorSet),
        getter("face", O::FontFace), setter("face", O::FontFaceSet),
        getter("size", O::FontSize), setter("size", O::FontSizeSet)
    ] },
    D { interface: I::HTMLFrameElement, name: "HTMLFrameElement", parent: Some(I::HTMLElement), constructor: None, members: &[
        getter("name", O::FrameName), setter("name", O::FrameNameSet),
        getter("scrolling", O::FrameScrolling), setter("scrolling", O::FrameScrollingSet),
        getter("src", O::FrameSrc), setter("src", O::FrameSrcSet),
        getter("frameBorder", O::FrameBorder), setter("frameBorder", O::FrameBorderSet),
        getter("longDesc", O::FrameLongDesc), setter("longDesc", O::FrameLongDescSet),
        getter("noResize", O::FrameNoResize), setter("noResize", O::FrameNoResizeSet),
        getter("marginHeight", O::FrameMarginHeight), setter("marginHeight", O::FrameMarginHeightSet),
        getter("marginWidth", O::FrameMarginWidth), setter("marginWidth", O::FrameMarginWidthSet)
    ] },
    D { interface: I::HTMLFrameSetElement, name: "HTMLFrameSetElement", parent: Some(I::HTMLElement), constructor: None, members: &[
        getter("cols", O::FrameSetCols), setter("cols", O::FrameSetColsSet),
        getter("rows", O::FrameSetRows), setter("rows", O::FrameSetRowsSet)
    ] },
    D { interface: I::HTMLUnknownElement, name: "HTMLUnknownElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLHtmlElement, name: "HTMLHtmlElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLHeadElement, name: "HTMLHeadElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLTitleElement, name: "HTMLTitleElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLBaseElement, name: "HTMLBaseElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLLinkElement, name: "HTMLLinkElement", parent: Some(I::HTMLElement), constructor: None, members: LINK },
    D { interface: I::HTMLMetaElement, name: "HTMLMetaElement", parent: Some(I::HTMLElement), constructor: None, members: META },
    D { interface: I::HTMLStyleElement, name: "HTMLStyleElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLBodyElement, name: "HTMLBodyElement", parent: Some(I::HTMLElement), constructor: None, members: HTML_BODY },
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
    D { interface: I::HTMLSpanElement, name: "HTMLSpanElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
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
    D { interface: I::HTMLFormElement, name: "HTMLFormElement", parent: Some(I::HTMLElement), constructor: None, members: FORM },
    D { interface: I::HTMLInputElement, name: "HTMLInputElement", parent: Some(I::HTMLElement), constructor: None, members: INPUT },
    D { interface: I::HTMLButtonElement, name: "HTMLButtonElement", parent: Some(I::HTMLElement), constructor: None, members: BUTTON },
    D { interface: I::HTMLSelectElement, name: "HTMLSelectElement", parent: Some(I::HTMLElement), constructor: None, members: SELECT },
    D { interface: I::HTMLOptionElement, name: "HTMLOptionElement", parent: Some(I::HTMLElement), constructor: None, members: OPTION },
    D { interface: I::HTMLOptGroupElement, name: "HTMLOptGroupElement", parent: Some(I::HTMLElement), constructor: None, members: OPTGROUP },
    D { interface: I::HTMLTextAreaElement, name: "HTMLTextAreaElement", parent: Some(I::HTMLElement), constructor: None, members: TEXTAREA },
    D { interface: I::HTMLLabelElement, name: "HTMLLabelElement", parent: Some(I::HTMLElement), constructor: None, members: LABEL },
    D { interface: I::HTMLFieldSetElement, name: "HTMLFieldSetElement", parent: Some(I::HTMLElement), constructor: None, members: FORM_CONTROL },
    D { interface: I::HTMLLegendElement, name: "HTMLLegendElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLDataListElement, name: "HTMLDataListElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLOutputElement, name: "HTMLOutputElement", parent: Some(I::HTMLElement), constructor: None, members: OUTPUT },
    D { interface: I::HTMLProgressElement, name: "HTMLProgressElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLMeterElement, name: "HTMLMeterElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLMediaElement, name: "HTMLMediaElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLVideoElement, name: "HTMLVideoElement", parent: Some(I::HTMLMediaElement), constructor: None, members: &[] },
    D { interface: I::HTMLAudioElement, name: "HTMLAudioElement", parent: Some(I::HTMLMediaElement), constructor: None, members: &[] },
    D { interface: I::HTMLSourceElement, name: "HTMLSourceElement", parent: Some(I::HTMLElement), constructor: None, members: SOURCE },
    D { interface: I::HTMLTrackElement, name: "HTMLTrackElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLCanvasElement, name: "HTMLCanvasElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLScriptElement, name: "HTMLScriptElement", parent: Some(I::HTMLElement), constructor: None, members: SCRIPT },
    D { interface: I::HTMLTemplateElement, name: "HTMLTemplateElement", parent: Some(I::HTMLElement), constructor: None, members: &[getter("content", O::TemplateContent)] },
    D { interface: I::HTMLSlotElement, name: "HTMLSlotElement", parent: Some(I::HTMLElement), constructor: None, members: SLOT },
    D { interface: I::HTMLIFrameElement, name: "HTMLIFrameElement", parent: Some(I::HTMLElement), constructor: None, members: IFRAME },
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
        members: &[getter("target", O::ProcessingInstructionTarget)],
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
    D { interface: I::DOMStringMap, name: "DOMStringMap", parent: None, constructor: None, members: &[] },
    D { interface: I::Selection, name: "Selection", parent: None, constructor: None, members: SELECTION },
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
    D { interface: I::HTMLFormControlsCollection, name: "HTMLFormControlsCollection", parent: Some(I::HTMLCollection), constructor: None, members: FORM_COLLECTION },
    D { interface: I::HTMLOptionsCollection, name: "HTMLOptionsCollection", parent: Some(I::HTMLCollection), constructor: None, members: OPTIONS_COLLECTION },
    D { interface: I::RadioNodeList, name: "RadioNodeList", parent: Some(I::NodeList), constructor: None, members: RADIO_LIST },
    D { interface: I::ValidityState, name: "ValidityState", parent: None, constructor: None, members: VALIDITY },
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
    D { interface: I::UIEvent, name: "UIEvent", parent: Some(I::Event),
        constructor: Some(O::UIEventConstructor), members: UI_EVENT },
    D { interface: I::FocusEvent, name: "FocusEvent", parent: Some(I::UIEvent),
        constructor: Some(O::FocusEventConstructor), members: FOCUS_EVENT },
    D { interface: I::MouseEvent, name: "MouseEvent", parent: Some(I::UIEvent),
        constructor: Some(O::MouseEventConstructor), members: MOUSE_EVENT },
    D { interface: I::WheelEvent, name: "WheelEvent", parent: Some(I::MouseEvent),
        constructor: Some(O::WheelEventConstructor), members: WHEEL_EVENT },
    D { interface: I::KeyboardEvent, name: "KeyboardEvent", parent: Some(I::UIEvent),
        constructor: Some(O::KeyboardEventConstructor), members: KEYBOARD_EVENT },
    D { interface: I::MessageEvent, name: "MessageEvent", parent: Some(I::Event),
        constructor: Some(O::MessageEventConstructor), members: MESSAGE_EVENT },
    D { interface: I::CompositionEvent, name: "CompositionEvent", parent: Some(I::UIEvent),
        constructor: Some(O::CompositionEventConstructor), members: COMPOSITION_EVENT },
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
        "span" => I::HTMLSpanElement,
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
        "dir" => I::HTMLDirectoryElement,
        "font" => I::HTMLFontElement,
        "frame" => I::HTMLFrameElement,
        "frameset" => I::HTMLFrameSetElement,
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
        "article" | "aside" | "b" | "bdi" | "bdo" | "big" | "center" | "cite" | "code" | "dd" | "dfn" | "dt" | "em" | "figcaption" | "figure" | "footer" | "header" | "hgroup" | "i" | "kbd" | "main" | "mark" | "nav" | "nobr" | "noembed" | "noframes" | "noscript" | "plaintext" | "rb" | "rp" | "rt" | "rtc" | "ruby" | "s" | "samp" | "search" | "section" | "small" | "strike" | "strong" | "sub" | "sup" | "tt" | "u" | "var" | "wbr" | "address" | "abbr" | "acronym" | "basefont" => I::HTMLElement,
        name if name.contains('-') => I::HTMLElement,
        _ => I::HTMLUnknownElement,
    }
}

/// DOM mixin members marked [Unscopable] in the DOM Standard. Each interface
/// receives its own null-prototype @@unscopables object.
pub fn unscopable_members(interface: I) -> &'static [&'static str] {
    match interface {
        I::Document | I::DocumentFragment => &["prepend", "append", "replaceChildren"],
        I::Element => &["prepend", "append", "replaceChildren", "before", "after",
            "replaceWith", "remove", "slot"],
        I::CharacterData | I::DocumentType => &["before", "after", "replaceWith", "remove"],
        _ => &[],
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
    host.install_static_method(I::URL, "canParse", O::URLCanParse, 1)?;
    host.install_static_method(I::URL, "parse", O::URLParse, 1)?;
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
    host.define_legacy_platform_object(InterfaceBrand::FORM_CONTROLS, LegacyPlatformHooks {
        supported_index_count: O::FormCollectionSupportedIndexCount,
        supported_names: Some(O::FormCollectionSupportedNames),
        indexed_getter: O::FormCollectionIndexedGetter, indexed_setter: None,
        named_getter: Some(O::FormCollectionNamedGetter), named_setter: None, named_deleter: None,
        override_builtins: false, unenumerable_names: true,
    })?;
    host.define_legacy_platform_object(InterfaceBrand::OPTIONS, LegacyPlatformHooks {
        supported_index_count: O::OptionsSupportedIndexCount,
        supported_names: Some(O::OptionsSupportedNames),
        indexed_getter: O::OptionsIndexedGetter, indexed_setter: Some(O::OptionsIndexedSetter),
        named_getter: Some(O::OptionsNamedGetter), named_setter: None, named_deleter: None,
        override_builtins: false, unenumerable_names: true,
    })?;
    host.define_legacy_platform_object(InterfaceBrand::RADIO_NODE_LIST, LegacyPlatformHooks {
        supported_index_count: O::RadioNodeListSupportedIndexCount,
        supported_names: None,
        indexed_getter: O::RadioNodeListIndexedGetter, indexed_setter: None,
        named_getter: None, named_setter: None, named_deleter: None,
        override_builtins: false, unenumerable_names: false,
    })?;
    for brand in [InterfaceBrand::NODE_LIST, InterfaceBrand::STATIC_NODE_LIST, InterfaceBrand::FILTERED_NODE_LIST] {
        host.define_legacy_platform_object(brand, LegacyPlatformHooks {
            supported_index_count: O::NodeListSupportedIndexCount, supported_names: None,
            indexed_getter: O::NodeListIndexedGetter, indexed_setter: None, named_getter: None,
            named_setter: None, named_deleter: None, override_builtins: false,
            unenumerable_names: false,
        })?;
    }
    host.define_legacy_platform_object(InterfaceBrand::NAMED_NODE_MAP, LegacyPlatformHooks {
        supported_index_count: O::NamedNodeMapSupportedIndexCount,
        supported_names: Some(O::NamedNodeMapSupportedNames),
        indexed_getter: O::NamedNodeMapItem, indexed_setter: None,
        named_getter: Some(O::NamedNodeMapGetNamedItem), named_setter: None, named_deleter: None,
        override_builtins: false, unenumerable_names: true,
    })?;
    host.define_legacy_platform_object(InterfaceBrand::DOM_TOKEN_LIST, LegacyPlatformHooks {
        supported_index_count: O::TokenListSupportedIndexCount, supported_names: None,
        indexed_getter: O::TokenListIndexedGetter, indexed_setter: None, named_getter: None,
        named_setter: None, named_deleter: None, override_builtins: false,
        unenumerable_names: false,
    })?;
    host.define_legacy_platform_object(InterfaceBrand::DOM_STRING_MAP, LegacyPlatformHooks {
        supported_index_count: O::DatasetIndexCount, supported_names: Some(O::DatasetSupportedNames),
        indexed_getter: O::DatasetIndexedGet, indexed_setter: None,
        named_getter: Some(O::DatasetGet), named_setter: Some(O::DatasetSet), named_deleter: Some(O::DatasetDelete),
        override_builtins: true, unenumerable_names: false,
    })?;
    host.define_iterable(I::NodeList, IterableKind::Values)?;
    host.define_iterable(I::DOMTokenList, IterableKind::Values)?;
    for interface in [I::URLSearchParams, I::FormData] {
        if host.define_pair_iterable(interface)? == OptionalHookInstall::NotSupported {
            report.not_installed.push(OptionalCapability::PairIterable(interface));
        }
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
    Ok(report)
}
