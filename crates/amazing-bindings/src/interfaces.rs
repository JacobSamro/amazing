use crate::host::LegacyReflectionKind as LR;
use amazing_dom::legacy_document::LegacyColor as LC;
use crate::host::{
    BindingHost, FormAttribute as F, Interface as I, InterfaceBrand, InterfaceDefinition as D, IterableKind, LegacyPlatformHooks, Member, MemberKind as K, Operation as O, OptionalHookInstall,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionalCapability {
    CssomGlobal,
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
    method("lookupPrefix", O::NodeLookupPrefix, 1),
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
    method("when", O::EventTargetWhen, 1),
];
const OBSERVABLE: &[Member] = &[
    method("subscribe", O::ObservableSubscribe, 0),
    method("map", O::ObservableMap, 1),
    method("filter", O::ObservableFilter, 1),
    method("take", O::ObservableTake, 1),
    method("drop", O::ObservableDrop, 1),
    method("flatMap", O::ObservableFlatMap, 1),
    method("switchMap", O::ObservableSwitchMap, 1),
    method("takeUntil", O::ObservableTakeUntil, 1),
    method("forEach", O::ObservableForEach, 1),
    method("toArray", O::ObservableToArray, 0),
    method("first", O::ObservableFirst, 0),
    method("last", O::ObservableLast, 0),
    method("find", O::ObservableFind, 1),
    method("some", O::ObservableSome, 1),
    method("every", O::ObservableEvery, 1),
    method("reduce", O::ObservableReduce, 1),
];
const SUBSCRIBER: &[Member] = &[
    method("next", O::SubscriberNext, 0),
    method("error", O::SubscriberError, 1),
    method("complete", O::SubscriberComplete, 0),
    method("addTeardown", O::SubscriberAddTeardown, 1),
    getter("active", O::SubscriberActive),
    getter("signal", O::SubscriberSignal),
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
const PROGRESS_EVENT: &[Member] = &[
    getter("lengthComputable", O::EventExtra("lengthComputable")),
    getter("loaded", O::EventExtra("loaded")),
    getter("total", O::EventExtra("total")),
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
    getter("dir", O::DocumentDir), setter("dir", O::DocumentSetDir),
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
    getter("onselectionchange", O::SelectionEventHandlerGet("selectionchange")), setter("onselectionchange", O::SelectionEventHandlerSet("selectionchange")),
    getter("onselectstart", O::SelectionEventHandlerGet("selectstart")), setter("onselectstart", O::SelectionEventHandlerSet("selectstart")),

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
const XML_SERIALIZER: &[Member] = &[
    method("serializeToString", O::XMLSerializerSerializeToString, 1),
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
    method("createContextualFragment", O::RangeCreateContextualFragment, 1),
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
    getter("type", O::SelectionType),
    getter("direction", O::SelectionDirection),
    method("getRangeAt", O::SelectionGetRangeAt, 1),
    method("addRange", O::SelectionAddRange, 1),
    method("removeRange", O::SelectionRemoveRange, 1),
    method("removeAllRanges", O::SelectionRemoveAllRanges, 0),
    method("empty", O::SelectionRemoveAllRanges, 0),
    method("collapse", O::SelectionCollapse, 1),
    method("setPosition", O::SelectionCollapse, 1),
    method("collapseToStart", O::SelectionCollapseToStart, 0),
    method("collapseToEnd", O::SelectionCollapseToEnd, 0),
    method("extend", O::SelectionExtend, 1),
    method("setBaseAndExtent", O::SelectionSetBaseAndExtent, 4),
    method("deleteFromDocument", O::SelectionDeleteFromDocument, 0),
    method("getComposedRanges", O::SelectionGetComposedRanges, 0),
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
    method("setHTMLUnsafe", O::ElementSetHTMLUnsafe, 1),
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
    // Animatable mixin (DOM-B-REFLECT): a minimal `animate()` -- see
    // `animation_bindings.rs` for scope.
    method("animate", O::ElementAnimate, 1),
    // ARIAMixin (DOM-B-REFLECT): `role` plus the aria-* string attribute
    // set -- see `aria_reflection.rs` for scope (element-reference
    // attributes such as `ariaLabelledByElements` are not implemented).
    getter("role", O::AriaGet("role")), setter("role", O::AriaSet("role")),
    getter("ariaAtomic", O::AriaGet("aria-atomic")), setter("ariaAtomic", O::AriaSet("aria-atomic")),
    getter("ariaAutoComplete", O::AriaGet("aria-autocomplete")), setter("ariaAutoComplete", O::AriaSet("aria-autocomplete")),
    getter("ariaBrailleLabel", O::AriaGet("aria-braillelabel")), setter("ariaBrailleLabel", O::AriaSet("aria-braillelabel")),
    getter("ariaBrailleRoleDescription", O::AriaGet("aria-brailleroledescription")), setter("ariaBrailleRoleDescription", O::AriaSet("aria-brailleroledescription")),
    getter("ariaBusy", O::AriaGet("aria-busy")), setter("ariaBusy", O::AriaSet("aria-busy")),
    getter("ariaChecked", O::AriaGet("aria-checked")), setter("ariaChecked", O::AriaSet("aria-checked")),
    getter("ariaColCount", O::AriaGet("aria-colcount")), setter("ariaColCount", O::AriaSet("aria-colcount")),
    getter("ariaColIndex", O::AriaGet("aria-colindex")), setter("ariaColIndex", O::AriaSet("aria-colindex")),
    getter("ariaColIndexText", O::AriaGet("aria-colindextext")), setter("ariaColIndexText", O::AriaSet("aria-colindextext")),
    getter("ariaColSpan", O::AriaGet("aria-colspan")), setter("ariaColSpan", O::AriaSet("aria-colspan")),
    getter("ariaCurrent", O::AriaGet("aria-current")), setter("ariaCurrent", O::AriaSet("aria-current")),
    getter("ariaDescription", O::AriaGet("aria-description")), setter("ariaDescription", O::AriaSet("aria-description")),
    getter("ariaDisabled", O::AriaGet("aria-disabled")), setter("ariaDisabled", O::AriaSet("aria-disabled")),
    getter("ariaExpanded", O::AriaGet("aria-expanded")), setter("ariaExpanded", O::AriaSet("aria-expanded")),
    getter("ariaHasPopup", O::AriaGet("aria-haspopup")), setter("ariaHasPopup", O::AriaSet("aria-haspopup")),
    getter("ariaHidden", O::AriaGet("aria-hidden")), setter("ariaHidden", O::AriaSet("aria-hidden")),
    getter("ariaInvalid", O::AriaGet("aria-invalid")), setter("ariaInvalid", O::AriaSet("aria-invalid")),
    getter("ariaKeyShortcuts", O::AriaGet("aria-keyshortcuts")), setter("ariaKeyShortcuts", O::AriaSet("aria-keyshortcuts")),
    getter("ariaLabel", O::AriaGet("aria-label")), setter("ariaLabel", O::AriaSet("aria-label")),
    getter("ariaLevel", O::AriaGet("aria-level")), setter("ariaLevel", O::AriaSet("aria-level")),
    getter("ariaLive", O::AriaGet("aria-live")), setter("ariaLive", O::AriaSet("aria-live")),
    getter("ariaModal", O::AriaGet("aria-modal")), setter("ariaModal", O::AriaSet("aria-modal")),
    getter("ariaMultiLine", O::AriaGet("aria-multiline")), setter("ariaMultiLine", O::AriaSet("aria-multiline")),
    getter("ariaMultiSelectable", O::AriaGet("aria-multiselectable")), setter("ariaMultiSelectable", O::AriaSet("aria-multiselectable")),
    getter("ariaOrientation", O::AriaGet("aria-orientation")), setter("ariaOrientation", O::AriaSet("aria-orientation")),
    getter("ariaPlaceholder", O::AriaGet("aria-placeholder")), setter("ariaPlaceholder", O::AriaSet("aria-placeholder")),
    getter("ariaPosInSet", O::AriaGet("aria-posinset")), setter("ariaPosInSet", O::AriaSet("aria-posinset")),
    getter("ariaPressed", O::AriaGet("aria-pressed")), setter("ariaPressed", O::AriaSet("aria-pressed")),
    getter("ariaReadOnly", O::AriaGet("aria-readonly")), setter("ariaReadOnly", O::AriaSet("aria-readonly")),
    getter("ariaRelevant", O::AriaGet("aria-relevant")), setter("ariaRelevant", O::AriaSet("aria-relevant")),
    getter("ariaRequired", O::AriaGet("aria-required")), setter("ariaRequired", O::AriaSet("aria-required")),
    getter("ariaRoleDescription", O::AriaGet("aria-roledescription")), setter("ariaRoleDescription", O::AriaSet("aria-roledescription")),
    getter("ariaRowCount", O::AriaGet("aria-rowcount")), setter("ariaRowCount", O::AriaSet("aria-rowcount")),
    getter("ariaRowIndex", O::AriaGet("aria-rowindex")), setter("ariaRowIndex", O::AriaSet("aria-rowindex")),
    getter("ariaRowIndexText", O::AriaGet("aria-rowindextext")), setter("ariaRowIndexText", O::AriaSet("aria-rowindextext")),
    getter("ariaRowSpan", O::AriaGet("aria-rowspan")), setter("ariaRowSpan", O::AriaSet("aria-rowspan")),
    getter("ariaSelected", O::AriaGet("aria-selected")), setter("ariaSelected", O::AriaSet("aria-selected")),
    getter("ariaSetSize", O::AriaGet("aria-setsize")), setter("ariaSetSize", O::AriaSet("aria-setsize")),
    getter("ariaSort", O::AriaGet("aria-sort")), setter("ariaSort", O::AriaSet("aria-sort")),
    getter("ariaValueMax", O::AriaGet("aria-valuemax")), setter("ariaValueMax", O::AriaSet("aria-valuemax")),
    getter("ariaValueMin", O::AriaGet("aria-valuemin")), setter("ariaValueMin", O::AriaSet("aria-valuemin")),
    getter("ariaValueNow", O::AriaGet("aria-valuenow")), setter("ariaValueNow", O::AriaSet("aria-valuenow")),
    getter("ariaValueText", O::AriaGet("aria-valuetext")), setter("ariaValueText", O::AriaSet("aria-valuetext")),
];
const ELEMENT_INTERNALS: &[Member] = &[
    getter("shadowRoot", O::InternalsShadowRoot),
    getter("form", O::InternalsForm),
    method("setFormValue", O::InternalsSetFormValue, 1),
    method("setValidity", O::InternalsSetValidity, 1),
    getter("willValidate", O::InternalsWillValidate),
    getter("validity", O::InternalsValidity),
    getter("validationMessage", O::InternalsValidationMessage),
    method("checkValidity", O::InternalsCheckValidity, 0),
    method("reportValidity", O::InternalsReportValidity, 0),
    getter("labels", O::InternalsLabels),
    getter("states", O::InternalsStates),
    getter("behaviors", O::InternalsBehaviors),
    getter("ariaActiveDescendantElement", O::InternalsAriaElementGet("ariaActiveDescendantElement")),
    setter("ariaActiveDescendantElement", O::InternalsAriaElementSet("ariaActiveDescendantElement")),
    getter("ariaControlsElements", O::InternalsAriaElementGet("ariaControlsElements")),
    setter("ariaControlsElements", O::InternalsAriaElementSet("ariaControlsElements")),
    getter("ariaDescribedByElements", O::InternalsAriaElementGet("ariaDescribedByElements")),
    setter("ariaDescribedByElements", O::InternalsAriaElementSet("ariaDescribedByElements")),
    getter("ariaDetailsElements", O::InternalsAriaElementGet("ariaDetailsElements")),
    setter("ariaDetailsElements", O::InternalsAriaElementSet("ariaDetailsElements")),
    getter("ariaErrorMessageElements", O::InternalsAriaElementGet("ariaErrorMessageElements")),
    setter("ariaErrorMessageElements", O::InternalsAriaElementSet("ariaErrorMessageElements")),
    getter("ariaFlowToElements", O::InternalsAriaElementGet("ariaFlowToElements")),
    setter("ariaFlowToElements", O::InternalsAriaElementSet("ariaFlowToElements")),
    getter("ariaLabelledByElements", O::InternalsAriaElementGet("ariaLabelledByElements")),
    setter("ariaLabelledByElements", O::InternalsAriaElementSet("ariaLabelledByElements")),
    getter("ariaOwnsElements", O::InternalsAriaElementGet("ariaOwnsElements")),
    setter("ariaOwnsElements", O::InternalsAriaElementSet("ariaOwnsElements")),
    getter("role", O::InternalsAriaGet("role")), setter("role", O::InternalsAriaSet("role")),
    getter("ariaAtomic", O::InternalsAriaGet("aria-atomic")), setter("ariaAtomic", O::InternalsAriaSet("aria-atomic")),
    getter("ariaAutoComplete", O::InternalsAriaGet("aria-autocomplete")), setter("ariaAutoComplete", O::InternalsAriaSet("aria-autocomplete")),
    getter("ariaBrailleLabel", O::InternalsAriaGet("aria-braillelabel")), setter("ariaBrailleLabel", O::InternalsAriaSet("aria-braillelabel")),
    getter("ariaBrailleRoleDescription", O::InternalsAriaGet("aria-brailleroledescription")), setter("ariaBrailleRoleDescription", O::InternalsAriaSet("aria-brailleroledescription")),
    getter("ariaBusy", O::InternalsAriaGet("aria-busy")), setter("ariaBusy", O::InternalsAriaSet("aria-busy")),
    getter("ariaChecked", O::InternalsAriaGet("aria-checked")), setter("ariaChecked", O::InternalsAriaSet("aria-checked")),
    getter("ariaColCount", O::InternalsAriaGet("aria-colcount")), setter("ariaColCount", O::InternalsAriaSet("aria-colcount")),
    getter("ariaColIndex", O::InternalsAriaGet("aria-colindex")), setter("ariaColIndex", O::InternalsAriaSet("aria-colindex")),
    getter("ariaColIndexText", O::InternalsAriaGet("aria-colindextext")), setter("ariaColIndexText", O::InternalsAriaSet("aria-colindextext")),
    getter("ariaColSpan", O::InternalsAriaGet("aria-colspan")), setter("ariaColSpan", O::InternalsAriaSet("aria-colspan")),
    getter("ariaCurrent", O::InternalsAriaGet("aria-current")), setter("ariaCurrent", O::InternalsAriaSet("aria-current")),
    getter("ariaDescription", O::InternalsAriaGet("aria-description")), setter("ariaDescription", O::InternalsAriaSet("aria-description")),
    getter("ariaDisabled", O::InternalsAriaGet("aria-disabled")), setter("ariaDisabled", O::InternalsAriaSet("aria-disabled")),
    getter("ariaExpanded", O::InternalsAriaGet("aria-expanded")), setter("ariaExpanded", O::InternalsAriaSet("aria-expanded")),
    getter("ariaHasPopup", O::InternalsAriaGet("aria-haspopup")), setter("ariaHasPopup", O::InternalsAriaSet("aria-haspopup")),
    getter("ariaHidden", O::InternalsAriaGet("aria-hidden")), setter("ariaHidden", O::InternalsAriaSet("aria-hidden")),
    getter("ariaInvalid", O::InternalsAriaGet("aria-invalid")), setter("ariaInvalid", O::InternalsAriaSet("aria-invalid")),
    getter("ariaKeyShortcuts", O::InternalsAriaGet("aria-keyshortcuts")), setter("ariaKeyShortcuts", O::InternalsAriaSet("aria-keyshortcuts")),
    getter("ariaLabel", O::InternalsAriaGet("aria-label")), setter("ariaLabel", O::InternalsAriaSet("aria-label")),
    getter("ariaLevel", O::InternalsAriaGet("aria-level")), setter("ariaLevel", O::InternalsAriaSet("aria-level")),
    getter("ariaLive", O::InternalsAriaGet("aria-live")), setter("ariaLive", O::InternalsAriaSet("aria-live")),
    getter("ariaModal", O::InternalsAriaGet("aria-modal")), setter("ariaModal", O::InternalsAriaSet("aria-modal")),
    getter("ariaMultiLine", O::InternalsAriaGet("aria-multiline")), setter("ariaMultiLine", O::InternalsAriaSet("aria-multiline")),
    getter("ariaMultiSelectable", O::InternalsAriaGet("aria-multiselectable")), setter("ariaMultiSelectable", O::InternalsAriaSet("aria-multiselectable")),
    getter("ariaOrientation", O::InternalsAriaGet("aria-orientation")), setter("ariaOrientation", O::InternalsAriaSet("aria-orientation")),
    getter("ariaPlaceholder", O::InternalsAriaGet("aria-placeholder")), setter("ariaPlaceholder", O::InternalsAriaSet("aria-placeholder")),
    getter("ariaPosInSet", O::InternalsAriaGet("aria-posinset")), setter("ariaPosInSet", O::InternalsAriaSet("aria-posinset")),
    getter("ariaPressed", O::InternalsAriaGet("aria-pressed")), setter("ariaPressed", O::InternalsAriaSet("aria-pressed")),
    getter("ariaReadOnly", O::InternalsAriaGet("aria-readonly")), setter("ariaReadOnly", O::InternalsAriaSet("aria-readonly")),
    getter("ariaRelevant", O::InternalsAriaGet("aria-relevant")), setter("ariaRelevant", O::InternalsAriaSet("aria-relevant")),
    getter("ariaRequired", O::InternalsAriaGet("aria-required")), setter("ariaRequired", O::InternalsAriaSet("aria-required")),
    getter("ariaRoleDescription", O::InternalsAriaGet("aria-roledescription")), setter("ariaRoleDescription", O::InternalsAriaSet("aria-roledescription")),
    getter("ariaRowCount", O::InternalsAriaGet("aria-rowcount")), setter("ariaRowCount", O::InternalsAriaSet("aria-rowcount")),
    getter("ariaRowIndex", O::InternalsAriaGet("aria-rowindex")), setter("ariaRowIndex", O::InternalsAriaSet("aria-rowindex")),
    getter("ariaRowIndexText", O::InternalsAriaGet("aria-rowindextext")), setter("ariaRowIndexText", O::InternalsAriaSet("aria-rowindextext")),
    getter("ariaRowSpan", O::InternalsAriaGet("aria-rowspan")), setter("ariaRowSpan", O::InternalsAriaSet("aria-rowspan")),
    getter("ariaSelected", O::InternalsAriaGet("aria-selected")), setter("ariaSelected", O::InternalsAriaSet("aria-selected")),
    getter("ariaSetSize", O::InternalsAriaGet("aria-setsize")), setter("ariaSetSize", O::InternalsAriaSet("aria-setsize")),
    getter("ariaSort", O::InternalsAriaGet("aria-sort")), setter("ariaSort", O::InternalsAriaSet("aria-sort")),
    getter("ariaValueMax", O::InternalsAriaGet("aria-valuemax")), setter("ariaValueMax", O::InternalsAriaSet("aria-valuemax")),
    getter("ariaValueMin", O::InternalsAriaGet("aria-valuemin")), setter("ariaValueMin", O::InternalsAriaSet("aria-valuemin")),
    getter("ariaValueNow", O::InternalsAriaGet("aria-valuenow")), setter("ariaValueNow", O::InternalsAriaSet("aria-valuenow")),
    getter("ariaValueText", O::InternalsAriaGet("aria-valuetext")), setter("ariaValueText", O::InternalsAriaSet("aria-valuetext")),
];
const CUSTOM_STATE_SET: &[Member] = &[
    getter("size", O::StateSetSize),
    method("add", O::StateSetAdd, 1), method("delete", O::StateSetDelete, 1),
    method("has", O::StateSetHas, 1), method("clear", O::StateSetClear, 0),
    method("forEach", O::StateSetForEach, 1), method("entries", O::StateSetEntries, 0),
    method("keys", O::StateSetValues, 0), method("values", O::StateSetValues, 0),
];
const ANIMATION: &[Member] = &[
    getter("finished", O::AnimationFinished),
    getter("ready", O::AnimationReady),
    method("cancel", O::AnimationCancel, 0),
    method("finish", O::AnimationFinish, 0),
    method("play", O::AnimationPlay, 0),
    method("pause", O::AnimationPause, 0),
    getter("onfinish", O::AnimationOnFinish), setter("onfinish", O::AnimationSetOnFinish),
];
const CLIPBOARD: &[Member] = &[
    method("readText", O::ClipboardReadText, 0),
    method("writeText", O::ClipboardWriteText, 1),
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
    method("setHTMLUnsafe", O::ElementSetHTMLUnsafe, 1),
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
#[path = "interfaces_reflection.rs"]
mod reflection_extra;
const HTML_ELEMENT_BASE: &[Member] = &[
    getter("onselectionchange", O::SelectionEventHandlerGet("selectionchange")), setter("onselectionchange", O::SelectionEventHandlerSet("selectionchange")),
    getter("onselectstart", O::SelectionEventHandlerGet("selectstart")), setter("onselectstart", O::SelectionEventHandlerSet("selectstart")),

    method("click", O::HTMLElementClick(false), 0),
    method("attachInternals", O::HTMLElementAttachInternals, 0),
    getter("onclick", O::HTMLElementOnClick), setter("onclick", O::HTMLElementSetOnClick),
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
    getter("origin", O::HyperlinkOrigin),
    getter("protocol", O::HyperlinkProtocol), setter("protocol", O::HyperlinkSetProtocol),
    getter("username", O::HyperlinkUsername), setter("username", O::HyperlinkSetUsername),
    getter("password", O::HyperlinkPassword), setter("password", O::HyperlinkSetPassword),
    getter("host", O::HyperlinkHost), setter("host", O::HyperlinkSetHost),
    getter("hostname", O::HyperlinkHostname), setter("hostname", O::HyperlinkSetHostname),
    getter("port", O::HyperlinkPort), setter("port", O::HyperlinkSetPort),
    getter("pathname", O::HyperlinkPathname), setter("pathname", O::HyperlinkSetPathname),
    getter("search", O::HyperlinkSearch), setter("search", O::HyperlinkSetSearch),
    getter("hash", O::HyperlinkHash), setter("hash", O::HyperlinkSetHash),
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
    getter("autocomplete", O::FormAutocompleteGet), setter("autocomplete", O::FormAutocompleteSet),
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
    getter("indeterminate", O::InputIndeterminate), setter("indeterminate", O::InputSetIndeterminate),
    getter("checked", O::InputChecked), setter("checked", O::InputSetChecked),
    getter("defaultChecked", O::InputDefaultChecked), setter("defaultChecked", O::InputSetDefaultChecked),
    getter("switch", O::LegacyReflectGet(I::HTMLInputElement, "switch", LR::Boolean)),
    setter("switch", O::LegacyReflectSet(I::HTMLInputElement, "switch", LR::Boolean)),
    getter("valueAsNumber", O::InputValueAsNumber), setter("valueAsNumber", O::InputSetValueAsNumber),
    getter("valueAsDate", O::InputValueAsDate), setter("valueAsDate", O::InputSetValueAsDate),
    method("stepUp", O::InputStepUp, 0), method("stepDown", O::InputStepDown, 0),
    getter("selectionStart", O::InputSelectionStart), setter("selectionStart", O::InputSetSelectionStart),
    getter("selectionEnd", O::InputSelectionEnd), setter("selectionEnd", O::InputSetSelectionEnd),
    getter("selectionDirection", O::InputSelectionDirection), setter("selectionDirection", O::InputSetSelectionDirection),
    method("select", O::InputSelect, 0), method("setSelectionRange", O::InputSetSelectionRange, 2),
    method("setRangeText", O::InputSetRangeText, 1),
    getter("willValidate", O::ControlWillValidate), getter("validity", O::ControlValidity),
    getter("validationMessage", O::ControlValidationMessage),
    method("checkValidity", O::ControlCheckValidity, 0), method("reportValidity", O::ControlReportValidity, 0),
    method("setCustomValidity", O::ControlSetCustomValidity, 1),
];
const FORM: &[Member] = &[
    form_get("acceptCharset", F::AcceptCharset), form_set("acceptCharset", F::AcceptCharset),
    form_get("action", F::Action), form_set("action", F::Action),
    getter("autocomplete", O::FormAutocompleteGet), setter("autocomplete", O::FormAutocompleteSet),
    form_get("enctype", F::Enctype), form_set("enctype", F::Enctype),
    form_get("encoding", F::Encoding), form_set("encoding", F::Encoding),
    form_get("method", F::Method), form_set("method", F::Method),
    form_get("name", F::Name), form_set("name", F::Name),
    form_get("noValidate", F::NoValidate), form_set("noValidate", F::NoValidate),
    form_get("target", F::Target), form_set("target", F::Target),
    form_get("rel", F::Rel), form_set("rel", F::Rel),
    form_get("relList", F::RelList), form_set("relList", F::RelList),
    getter("elements", O::FormElements), getter("length", O::FormLength),
    method("submit", O::FormSubmit, 0), method("requestSubmit", O::FormRequestSubmit, 0),
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
const FIELDSET: &[Member] = &[
    form_get("disabled", F::Disabled), form_set("disabled", F::Disabled),
    getter("form", O::FormControlForm), getter("name", O::FormControlName), setter("name", O::FormControlSetName),
    getter("elements", O::FieldsetElements),
    getter("willValidate", O::ControlWillValidate), getter("validity", O::ControlValidity),
    getter("validationMessage", O::ControlValidationMessage),
    method("checkValidity", O::ControlCheckValidity, 0), method("reportValidity", O::ControlReportValidity, 0),
    method("setCustomValidity", O::ControlSetCustomValidity, 1),
];
const OUTPUT: &[Member] = &[
    getter("htmlFor", O::OutputHtmlFor), setter("htmlFor", O::OutputSetHtmlFor),
    getter("value", O::OutputValue), setter("value", O::OutputSetValue),
    getter("defaultValue", O::OutputDefaultValue), setter("defaultValue", O::OutputSetDefaultValue),
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
    getter("autocomplete", O::FormAutocompleteGet), setter("autocomplete", O::FormAutocompleteSet),
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
    method("setRangeText", O::InputSetRangeText, 1),
    getter("willValidate", O::ControlWillValidate), getter("validity", O::ControlValidity),
    getter("validationMessage", O::ControlValidationMessage),
    method("checkValidity", O::ControlCheckValidity, 0), method("reportValidity", O::ControlReportValidity, 0),
    method("setCustomValidity", O::ControlSetCustomValidity, 1),
];
const SELECT: &[Member] = &[
    getter("autocomplete", O::FormAutocompleteGet), setter("autocomplete", O::FormAutocompleteSet),
    form_get("disabled", F::Disabled), form_set("disabled", F::Disabled),
    form_get("name", F::Name), form_set("name", F::Name),
    form_get("required", F::Required), form_set("required", F::Required),
    form_get("size", F::Size), form_set("size", F::Size),
    getter("form", O::FormControlForm), getter("value", O::SelectValue), setter("value", O::SelectSetValue),
    getter("selectedIndex", O::SelectSelectedIndex), setter("selectedIndex", O::SelectSetSelectedIndex),
    getter("options", O::SelectOptions), getter("length", O::SelectLength),
    setter("length", O::SelectSetLength),
    getter("selectedOptions", O::SelectSelectedOptions),
    getter("multiple", O::SelectMultiple), setter("multiple", O::SelectSetMultiple),
    method("add", O::SelectAdd, 1), method("remove", O::SelectRemove, 0),
    method("item", O::SelectItem, 1), method("namedItem", O::SelectNamedItem, 1),
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
    getter("index", O::OptionIndex),
];
const OPTGROUP: &[Member] = &[
    form_get("disabled", F::Disabled), form_set("disabled", F::Disabled),
    form_get("label", F::Label), form_set("label", F::Label),
];
const LABEL: &[Member] = &[
    form_get("htmlFor", F::HtmlFor), form_set("htmlFor", F::HtmlFor),
    getter("form", O::LabelForm), getter("control", O::LabelControl),
];
const OBJ: &[Member] = &[getter("form", O::FormControlForm)];
const PROGRESS: &[Member] = &[
    getter("value", O::ProgressValue), setter("value", O::ProgressSetValue),
    getter("max", O::ProgressMax), setter("max", O::ProgressSetMax),
    getter("position", O::ProgressPosition),
];
const METER: &[Member] = &[
    getter("value", O::MeterValue), setter("value", O::MeterSetValue),
    getter("min", O::MeterMin), setter("min", O::MeterSetMin),
    getter("max", O::MeterMax), setter("max", O::MeterSetMax),
    getter("low", O::MeterLow), setter("low", O::MeterSetLow),
    getter("high", O::MeterHigh), setter("high", O::MeterSetHigh),
    getter("optimum", O::MeterOptimum), setter("optimum", O::MeterSetOptimum),
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
    getter("text", O::ScriptText), setter("text", O::ScriptSetText),
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
    getter("cellIndex", O::CellIndex),
    getter("colSpan", O::CellColSpan), setter("colSpan", O::CellSetColSpan),
    getter("rowSpan", O::CellRowSpan), setter("rowSpan", O::CellSetRowSpan),
];
const OLIST: &[Member] = &[
    getter("start", O::OListStart), setter("start", O::OListSetStart),
    getter("reversed", O::OListReversed), setter("reversed", O::OListSetReversed),
];
const LI: &[Member] = &[getter("value", O::LIValue), setter("value", O::LISetValue)];
const EMPTY_LEGACY_BASE: &[Member] = &[];
const HTML_ELEMENT: [Member; HTML_ELEMENT_BASE.len() + reflection_extra::HTML_ELEMENT_NEW.len()] =
    with_legacy_members(HTML_ELEMENT_BASE, reflection_extra::HTML_ELEMENT_NEW);
const fn with_legacy_members<const N: usize>(base: &[Member], extra: &[Member]) -> [Member; N] {
    assert!(N == base.len() + extra.len());
    let mut members = [getter("", O::NodeType); N];
    let mut index = 0;
    while index < base.len() { members[index] = base[index]; index += 1; }
    let mut extra_index = 0;
    while extra_index < extra.len() { members[index] = extra[extra_index]; index += 1; extra_index += 1; }
    members
}
const LEGACY_ANCHOR_EXTRA: &[Member] = &[
    getter("coords", O::LegacyReflectGet(I::HTMLAnchorElement, "coords", LR::String)), setter("coords", O::LegacyReflectSet(I::HTMLAnchorElement, "coords", LR::String)),
    getter("charset", O::LegacyReflectGet(I::HTMLAnchorElement, "charset", LR::String)), setter("charset", O::LegacyReflectSet(I::HTMLAnchorElement, "charset", LR::String)),
    getter("name", O::LegacyReflectGet(I::HTMLAnchorElement, "name", LR::String)), setter("name", O::LegacyReflectSet(I::HTMLAnchorElement, "name", LR::String)),
    getter("rev", O::LegacyReflectGet(I::HTMLAnchorElement, "rev", LR::String)), setter("rev", O::LegacyReflectSet(I::HTMLAnchorElement, "rev", LR::String)),
    getter("shape", O::LegacyReflectGet(I::HTMLAnchorElement, "shape", LR::String)), setter("shape", O::LegacyReflectSet(I::HTMLAnchorElement, "shape", LR::String)),
];
const LEGACY_ANCHOR: [Member; (HYPERLINK).len() + LEGACY_ANCHOR_EXTRA.len()] = with_legacy_members(HYPERLINK, LEGACY_ANCHOR_EXTRA);
const LEGACY_AREA_EXTRA: &[Member] = &[
    getter("noHref", O::LegacyReflectGet(I::HTMLAreaElement, "nohref", LR::Boolean)), setter("noHref", O::LegacyReflectSet(I::HTMLAreaElement, "nohref", LR::Boolean)),
];
const LEGACY_AREA: [Member; (HYPERLINK).len() + LEGACY_AREA_EXTRA.len()] = with_legacy_members(HYPERLINK, LEGACY_AREA_EXTRA);
const LEGACY_BODY_EXTRA: &[Member] = &[
    getter("background", O::LegacyReflectGet(I::HTMLBodyElement, "background", LR::String)), setter("background", O::LegacyReflectSet(I::HTMLBodyElement, "background", LR::String)),
];
const LEGACY_BODY: [Member; (HTML_BODY).len() + LEGACY_BODY_EXTRA.len()] = with_legacy_members(HTML_BODY, LEGACY_BODY_EXTRA);
const LEGACY_BR_EXTRA: &[Member] = &[
    getter("clear", O::LegacyReflectGet(I::HTMLBRElement, "clear", LR::String)), setter("clear", O::LegacyReflectSet(I::HTMLBRElement, "clear", LR::String)),
];
const LEGACY_BR: [Member; (EMPTY_LEGACY_BASE).len() + LEGACY_BR_EXTRA.len()] = with_legacy_members(EMPTY_LEGACY_BASE, LEGACY_BR_EXTRA);
const LEGACY_TABLECAPTION_EXTRA: &[Member] = &[
    getter("align", O::LegacyReflectGet(I::HTMLTableCaptionElement, "align", LR::String)), setter("align", O::LegacyReflectSet(I::HTMLTableCaptionElement, "align", LR::String)),
];
const LEGACY_TABLECAPTION: [Member; (EMPTY_LEGACY_BASE).len() + LEGACY_TABLECAPTION_EXTRA.len()] = with_legacy_members(EMPTY_LEGACY_BASE, LEGACY_TABLECAPTION_EXTRA);
const LEGACY_TABLECOL_EXTRA: &[Member] = &[
    getter("align", O::LegacyReflectGet(I::HTMLTableColElement, "align", LR::String)), setter("align", O::LegacyReflectSet(I::HTMLTableColElement, "align", LR::String)),
    getter("ch", O::LegacyReflectGet(I::HTMLTableColElement, "char", LR::String)), setter("ch", O::LegacyReflectSet(I::HTMLTableColElement, "char", LR::String)),
    getter("chOff", O::LegacyReflectGet(I::HTMLTableColElement, "charoff", LR::String)), setter("chOff", O::LegacyReflectSet(I::HTMLTableColElement, "charoff", LR::String)),
    getter("vAlign", O::LegacyReflectGet(I::HTMLTableColElement, "valign", LR::String)), setter("vAlign", O::LegacyReflectSet(I::HTMLTableColElement, "valign", LR::String)),
    getter("width", O::LegacyReflectGet(I::HTMLTableColElement, "width", LR::String)), setter("width", O::LegacyReflectSet(I::HTMLTableColElement, "width", LR::String)),
];
const LEGACY_TABLECOL: [Member; (EMPTY_LEGACY_BASE).len() + LEGACY_TABLECOL_EXTRA.len()] = with_legacy_members(EMPTY_LEGACY_BASE, LEGACY_TABLECOL_EXTRA);
const LEGACY_DIV_EXTRA: &[Member] = &[
    getter("align", O::LegacyReflectGet(I::HTMLDivElement, "align", LR::String)), setter("align", O::LegacyReflectSet(I::HTMLDivElement, "align", LR::String)),
];
const LEGACY_DIV: [Member; (EMPTY_LEGACY_BASE).len() + LEGACY_DIV_EXTRA.len()] = with_legacy_members(EMPTY_LEGACY_BASE, LEGACY_DIV_EXTRA);
const LEGACY_DLIST_EXTRA: &[Member] = &[
    getter("compact", O::LegacyReflectGet(I::HTMLDListElement, "compact", LR::Boolean)), setter("compact", O::LegacyReflectSet(I::HTMLDListElement, "compact", LR::Boolean)),
];
const LEGACY_DLIST: [Member; (EMPTY_LEGACY_BASE).len() + LEGACY_DLIST_EXTRA.len()] = with_legacy_members(EMPTY_LEGACY_BASE, LEGACY_DLIST_EXTRA);
const LEGACY_EMBED_EXTRA: &[Member] = &[
    getter("align", O::LegacyReflectGet(I::HTMLEmbedElement, "align", LR::String)), setter("align", O::LegacyReflectSet(I::HTMLEmbedElement, "align", LR::String)),
    getter("name", O::LegacyReflectGet(I::HTMLEmbedElement, "name", LR::String)), setter("name", O::LegacyReflectSet(I::HTMLEmbedElement, "name", LR::String)),
];
const LEGACY_EMBED: [Member; (EMPTY_LEGACY_BASE).len() + LEGACY_EMBED_EXTRA.len()] = with_legacy_members(EMPTY_LEGACY_BASE, LEGACY_EMBED_EXTRA);
const LEGACY_HEADING_EXTRA: &[Member] = &[
    getter("align", O::LegacyReflectGet(I::HTMLHeadingElement, "align", LR::String)), setter("align", O::LegacyReflectSet(I::HTMLHeadingElement, "align", LR::String)),
];
const LEGACY_HEADING: [Member; (EMPTY_LEGACY_BASE).len() + LEGACY_HEADING_EXTRA.len()] = with_legacy_members(EMPTY_LEGACY_BASE, LEGACY_HEADING_EXTRA);
const LEGACY_HR_EXTRA: &[Member] = &[
    getter("align", O::LegacyReflectGet(I::HTMLHRElement, "align", LR::String)), setter("align", O::LegacyReflectSet(I::HTMLHRElement, "align", LR::String)),
    getter("color", O::LegacyReflectGet(I::HTMLHRElement, "color", LR::String)), setter("color", O::LegacyReflectSet(I::HTMLHRElement, "color", LR::String)),
    getter("noShade", O::LegacyReflectGet(I::HTMLHRElement, "noshade", LR::Boolean)), setter("noShade", O::LegacyReflectSet(I::HTMLHRElement, "noshade", LR::Boolean)),
    getter("size", O::LegacyReflectGet(I::HTMLHRElement, "size", LR::String)), setter("size", O::LegacyReflectSet(I::HTMLHRElement, "size", LR::String)),
    getter("width", O::LegacyReflectGet(I::HTMLHRElement, "width", LR::String)), setter("width", O::LegacyReflectSet(I::HTMLHRElement, "width", LR::String)),
];
const LEGACY_HR: [Member; (EMPTY_LEGACY_BASE).len() + LEGACY_HR_EXTRA.len()] = with_legacy_members(EMPTY_LEGACY_BASE, LEGACY_HR_EXTRA);
const LEGACY_HTML_EXTRA: &[Member] = &[
    getter("version", O::LegacyReflectGet(I::HTMLHtmlElement, "version", LR::String)), setter("version", O::LegacyReflectSet(I::HTMLHtmlElement, "version", LR::String)),
];
const LEGACY_HTML: [Member; (EMPTY_LEGACY_BASE).len() + LEGACY_HTML_EXTRA.len()] = with_legacy_members(EMPTY_LEGACY_BASE, LEGACY_HTML_EXTRA);
const LEGACY_IFRAME_EXTRA: &[Member] = &[
    getter("align", O::LegacyReflectGet(I::HTMLIFrameElement, "align", LR::String)), setter("align", O::LegacyReflectSet(I::HTMLIFrameElement, "align", LR::String)),
    getter("scrolling", O::LegacyReflectGet(I::HTMLIFrameElement, "scrolling", LR::String)), setter("scrolling", O::LegacyReflectSet(I::HTMLIFrameElement, "scrolling", LR::String)),
    getter("frameBorder", O::LegacyReflectGet(I::HTMLIFrameElement, "frameborder", LR::String)), setter("frameBorder", O::LegacyReflectSet(I::HTMLIFrameElement, "frameborder", LR::String)),
    getter("longDesc", O::LegacyReflectGet(I::HTMLIFrameElement, "longdesc", LR::UsvUrl)), setter("longDesc", O::LegacyReflectSet(I::HTMLIFrameElement, "longdesc", LR::UsvUrl)),
    getter("marginHeight", O::LegacyReflectGet(I::HTMLIFrameElement, "marginheight", LR::LegacyString)), setter("marginHeight", O::LegacyReflectSet(I::HTMLIFrameElement, "marginheight", LR::LegacyString)),
    getter("marginWidth", O::LegacyReflectGet(I::HTMLIFrameElement, "marginwidth", LR::LegacyString)), setter("marginWidth", O::LegacyReflectSet(I::HTMLIFrameElement, "marginwidth", LR::LegacyString)),
];
const LEGACY_IFRAME: [Member; (IFRAME).len() + LEGACY_IFRAME_EXTRA.len()] = with_legacy_members(IFRAME, LEGACY_IFRAME_EXTRA);
const LEGACY_IMAGE_EXTRA: &[Member] = &[
    getter("name", O::LegacyReflectGet(I::HTMLImageElement, "name", LR::String)), setter("name", O::LegacyReflectSet(I::HTMLImageElement, "name", LR::String)),
    getter("lowsrc", O::LegacyReflectGet(I::HTMLImageElement, "lowsrc", LR::UsvUrl)), setter("lowsrc", O::LegacyReflectSet(I::HTMLImageElement, "lowsrc", LR::UsvUrl)),
    getter("align", O::LegacyReflectGet(I::HTMLImageElement, "align", LR::String)), setter("align", O::LegacyReflectSet(I::HTMLImageElement, "align", LR::String)),
    getter("hspace", O::LegacyReflectGet(I::HTMLImageElement, "hspace", LR::Unsigned)), setter("hspace", O::LegacyReflectSet(I::HTMLImageElement, "hspace", LR::Unsigned)),
    getter("vspace", O::LegacyReflectGet(I::HTMLImageElement, "vspace", LR::Unsigned)), setter("vspace", O::LegacyReflectSet(I::HTMLImageElement, "vspace", LR::Unsigned)),
    getter("longDesc", O::LegacyReflectGet(I::HTMLImageElement, "longdesc", LR::UsvUrl)), setter("longDesc", O::LegacyReflectSet(I::HTMLImageElement, "longdesc", LR::UsvUrl)),
    getter("border", O::LegacyReflectGet(I::HTMLImageElement, "border", LR::LegacyString)), setter("border", O::LegacyReflectSet(I::HTMLImageElement, "border", LR::LegacyString)),
];
const LEGACY_IMAGE: [Member; (IMAGE).len() + LEGACY_IMAGE_EXTRA.len()] = with_legacy_members(IMAGE, LEGACY_IMAGE_EXTRA);
const LEGACY_INPUT_EXTRA: &[Member] = &[
    getter("align", O::LegacyReflectGet(I::HTMLInputElement, "align", LR::String)), setter("align", O::LegacyReflectSet(I::HTMLInputElement, "align", LR::String)),
    getter("useMap", O::LegacyReflectGet(I::HTMLInputElement, "usemap", LR::String)), setter("useMap", O::LegacyReflectSet(I::HTMLInputElement, "usemap", LR::String)),
];
const LEGACY_INPUT: [Member; (INPUT).len() + LEGACY_INPUT_EXTRA.len()] = with_legacy_members(INPUT, LEGACY_INPUT_EXTRA);
const LEGACY_LEGEND_EXTRA: &[Member] = &[
    getter("align", O::LegacyReflectGet(I::HTMLLegendElement, "align", LR::String)), setter("align", O::LegacyReflectSet(I::HTMLLegendElement, "align", LR::String)),
];
const LEGACY_LEGEND: [Member; (EMPTY_LEGACY_BASE).len() + LEGACY_LEGEND_EXTRA.len()] = with_legacy_members(EMPTY_LEGACY_BASE, LEGACY_LEGEND_EXTRA);
const LEGACY_LI_EXTRA: &[Member] = &[
    getter("type", O::LegacyReflectGet(I::HTMLLIElement, "type", LR::String)), setter("type", O::LegacyReflectSet(I::HTMLLIElement, "type", LR::String)),
];
const LEGACY_LI: [Member; (LI).len() + LEGACY_LI_EXTRA.len()] = with_legacy_members(LI, LEGACY_LI_EXTRA);
const LEGACY_LINK_EXTRA: &[Member] = &[
    getter("charset", O::LegacyReflectGet(I::HTMLLinkElement, "charset", LR::String)), setter("charset", O::LegacyReflectSet(I::HTMLLinkElement, "charset", LR::String)),
    getter("rev", O::LegacyReflectGet(I::HTMLLinkElement, "rev", LR::String)), setter("rev", O::LegacyReflectSet(I::HTMLLinkElement, "rev", LR::String)),
    getter("target", O::LegacyReflectGet(I::HTMLLinkElement, "target", LR::String)), setter("target", O::LegacyReflectSet(I::HTMLLinkElement, "target", LR::String)),
];
const LEGACY_LINK: [Member; (LINK).len() + LEGACY_LINK_EXTRA.len()] = with_legacy_members(LINK, LEGACY_LINK_EXTRA);
const LEGACY_MENU_EXTRA: &[Member] = &[
    getter("compact", O::LegacyReflectGet(I::HTMLMenuElement, "compact", LR::Boolean)), setter("compact", O::LegacyReflectSet(I::HTMLMenuElement, "compact", LR::Boolean)),
];
const LEGACY_MENU: [Member; (EMPTY_LEGACY_BASE).len() + LEGACY_MENU_EXTRA.len()] = with_legacy_members(EMPTY_LEGACY_BASE, LEGACY_MENU_EXTRA);
const LEGACY_META_EXTRA: &[Member] = &[
    getter("scheme", O::LegacyReflectGet(I::HTMLMetaElement, "scheme", LR::String)), setter("scheme", O::LegacyReflectSet(I::HTMLMetaElement, "scheme", LR::String)),
];
const LEGACY_META: [Member; (META).len() + LEGACY_META_EXTRA.len()] = with_legacy_members(META, LEGACY_META_EXTRA);
const LEGACY_OBJECT_EXTRA: &[Member] = &[
    getter("align", O::LegacyReflectGet(I::HTMLObjectElement, "align", LR::String)), setter("align", O::LegacyReflectSet(I::HTMLObjectElement, "align", LR::String)),
    getter("archive", O::LegacyReflectGet(I::HTMLObjectElement, "archive", LR::String)), setter("archive", O::LegacyReflectSet(I::HTMLObjectElement, "archive", LR::String)),
    getter("code", O::LegacyReflectGet(I::HTMLObjectElement, "code", LR::String)), setter("code", O::LegacyReflectSet(I::HTMLObjectElement, "code", LR::String)),
    getter("declare", O::LegacyReflectGet(I::HTMLObjectElement, "declare", LR::Boolean)), setter("declare", O::LegacyReflectSet(I::HTMLObjectElement, "declare", LR::Boolean)),
    getter("hspace", O::LegacyReflectGet(I::HTMLObjectElement, "hspace", LR::Unsigned)), setter("hspace", O::LegacyReflectSet(I::HTMLObjectElement, "hspace", LR::Unsigned)),
    getter("standby", O::LegacyReflectGet(I::HTMLObjectElement, "standby", LR::String)), setter("standby", O::LegacyReflectSet(I::HTMLObjectElement, "standby", LR::String)),
    getter("vspace", O::LegacyReflectGet(I::HTMLObjectElement, "vspace", LR::Unsigned)), setter("vspace", O::LegacyReflectSet(I::HTMLObjectElement, "vspace", LR::Unsigned)),
    getter("codeBase", O::LegacyReflectGet(I::HTMLObjectElement, "codebase", LR::Url)), setter("codeBase", O::LegacyReflectSet(I::HTMLObjectElement, "codebase", LR::Url)),
    getter("codeType", O::LegacyReflectGet(I::HTMLObjectElement, "codetype", LR::String)), setter("codeType", O::LegacyReflectSet(I::HTMLObjectElement, "codetype", LR::String)),
    getter("useMap", O::LegacyReflectGet(I::HTMLObjectElement, "usemap", LR::String)), setter("useMap", O::LegacyReflectSet(I::HTMLObjectElement, "usemap", LR::String)),
    getter("border", O::LegacyReflectGet(I::HTMLObjectElement, "border", LR::LegacyString)), setter("border", O::LegacyReflectSet(I::HTMLObjectElement, "border", LR::LegacyString)),
];
// `OBJ` gained a `form` getter (form-associated custom element work) after
// this legacy-attribute set was first written; base on it, not on the empty
// legacy base, so a merge with that upstream change keeps both.
const LEGACY_OBJECT: [Member; (OBJ).len() + LEGACY_OBJECT_EXTRA.len()] = with_legacy_members(OBJ, LEGACY_OBJECT_EXTRA);
const LEGACY_OLIST_EXTRA: &[Member] = &[
    getter("compact", O::LegacyReflectGet(I::HTMLOListElement, "compact", LR::Boolean)), setter("compact", O::LegacyReflectSet(I::HTMLOListElement, "compact", LR::Boolean)),
];
const LEGACY_OLIST: [Member; (OLIST).len() + LEGACY_OLIST_EXTRA.len()] = with_legacy_members(OLIST, LEGACY_OLIST_EXTRA);
const LEGACY_PARAGRAPH_EXTRA: &[Member] = &[
    getter("align", O::LegacyReflectGet(I::HTMLParagraphElement, "align", LR::String)), setter("align", O::LegacyReflectSet(I::HTMLParagraphElement, "align", LR::String)),
];
const LEGACY_PARAGRAPH: [Member; (EMPTY_LEGACY_BASE).len() + LEGACY_PARAGRAPH_EXTRA.len()] = with_legacy_members(EMPTY_LEGACY_BASE, LEGACY_PARAGRAPH_EXTRA);
const LEGACY_PARAM_EXTRA: &[Member] = &[
    getter("name", O::LegacyReflectGet(I::HTMLParamElement, "name", LR::String)), setter("name", O::LegacyReflectSet(I::HTMLParamElement, "name", LR::String)),
    getter("value", O::LegacyReflectGet(I::HTMLParamElement, "value", LR::String)), setter("value", O::LegacyReflectSet(I::HTMLParamElement, "value", LR::String)),
    getter("type", O::LegacyReflectGet(I::HTMLParamElement, "type", LR::String)), setter("type", O::LegacyReflectSet(I::HTMLParamElement, "type", LR::String)),
    getter("valueType", O::LegacyReflectGet(I::HTMLParamElement, "valuetype", LR::String)), setter("valueType", O::LegacyReflectSet(I::HTMLParamElement, "valuetype", LR::String)),
];
const LEGACY_PARAM: [Member; (EMPTY_LEGACY_BASE).len() + LEGACY_PARAM_EXTRA.len()] = with_legacy_members(EMPTY_LEGACY_BASE, LEGACY_PARAM_EXTRA);
const LEGACY_PRE_EXTRA: &[Member] = &[
    getter("width", O::LegacyReflectGet(I::HTMLPreElement, "width", LR::Signed)), setter("width", O::LegacyReflectSet(I::HTMLPreElement, "width", LR::Signed)),
];
const LEGACY_PRE: [Member; (EMPTY_LEGACY_BASE).len() + LEGACY_PRE_EXTRA.len()] = with_legacy_members(EMPTY_LEGACY_BASE, LEGACY_PRE_EXTRA);
const LEGACY_STYLE_EXTRA: &[Member] = &[
    getter("type", O::LegacyReflectGet(I::HTMLStyleElement, "type", LR::String)), setter("type", O::LegacyReflectSet(I::HTMLStyleElement, "type", LR::String)),
];
const LEGACY_STYLE: [Member; (EMPTY_LEGACY_BASE).len() + LEGACY_STYLE_EXTRA.len()] = with_legacy_members(EMPTY_LEGACY_BASE, LEGACY_STYLE_EXTRA);
const LEGACY_SCRIPT_EXTRA: &[Member] = &[
    getter("charset", O::LegacyReflectGet(I::HTMLScriptElement, "charset", LR::String)), setter("charset", O::LegacyReflectSet(I::HTMLScriptElement, "charset", LR::String)),
    getter("event", O::LegacyReflectGet(I::HTMLScriptElement, "event", LR::String)), setter("event", O::LegacyReflectSet(I::HTMLScriptElement, "event", LR::String)),
    getter("htmlFor", O::LegacyReflectGet(I::HTMLScriptElement, "for", LR::String)), setter("htmlFor", O::LegacyReflectSet(I::HTMLScriptElement, "for", LR::String)),
];
const LEGACY_SCRIPT: [Member; (SCRIPT).len() + LEGACY_SCRIPT_EXTRA.len()] = with_legacy_members(SCRIPT, LEGACY_SCRIPT_EXTRA);
const LEGACY_TABLE_EXTRA: &[Member] = &[
    getter("caption", O::TableCaption), setter("caption", O::TableSetCaption),
    method("createCaption", O::TableCreateCaption, 0), method("deleteCaption", O::TableDeleteCaption, 0),
    getter("tHead", O::TableHead), setter("tHead", O::TableSetHead),
    method("createTHead", O::TableCreateHead, 0), method("deleteTHead", O::TableDeleteHead, 0),
    getter("tFoot", O::TableFoot), setter("tFoot", O::TableSetFoot),
    method("createTFoot", O::TableCreateFoot, 0), method("deleteTFoot", O::TableDeleteFoot, 0),
    getter("tBodies", O::TableBodies), method("createTBody", O::TableCreateBody, 0),
    getter("rows", O::TableRows), method("insertRow", O::TableInsertRow, 0), method("deleteRow", O::TableDeleteRow, 1),
    getter("align", O::LegacyReflectGet(I::HTMLTableElement, "align", LR::String)), setter("align", O::LegacyReflectSet(I::HTMLTableElement, "align", LR::String)),
    getter("border", O::LegacyReflectGet(I::HTMLTableElement, "border", LR::String)), setter("border", O::LegacyReflectSet(I::HTMLTableElement, "border", LR::String)),
    getter("frame", O::LegacyReflectGet(I::HTMLTableElement, "frame", LR::String)), setter("frame", O::LegacyReflectSet(I::HTMLTableElement, "frame", LR::String)),
    getter("rules", O::LegacyReflectGet(I::HTMLTableElement, "rules", LR::String)), setter("rules", O::LegacyReflectSet(I::HTMLTableElement, "rules", LR::String)),
    getter("summary", O::LegacyReflectGet(I::HTMLTableElement, "summary", LR::String)), setter("summary", O::LegacyReflectSet(I::HTMLTableElement, "summary", LR::String)),
    getter("width", O::LegacyReflectGet(I::HTMLTableElement, "width", LR::String)), setter("width", O::LegacyReflectSet(I::HTMLTableElement, "width", LR::String)),
    getter("bgColor", O::LegacyReflectGet(I::HTMLTableElement, "bgcolor", LR::LegacyString)), setter("bgColor", O::LegacyReflectSet(I::HTMLTableElement, "bgcolor", LR::LegacyString)),
    getter("cellPadding", O::LegacyReflectGet(I::HTMLTableElement, "cellpadding", LR::LegacyString)), setter("cellPadding", O::LegacyReflectSet(I::HTMLTableElement, "cellpadding", LR::LegacyString)),
    getter("cellSpacing", O::LegacyReflectGet(I::HTMLTableElement, "cellspacing", LR::LegacyString)), setter("cellSpacing", O::LegacyReflectSet(I::HTMLTableElement, "cellspacing", LR::LegacyString)),
];
const LEGACY_TABLE: [Member; (EMPTY_LEGACY_BASE).len() + LEGACY_TABLE_EXTRA.len()] = with_legacy_members(EMPTY_LEGACY_BASE, LEGACY_TABLE_EXTRA);
const LEGACY_TABLESECTION_EXTRA: &[Member] = &[
    getter("rows", O::SectionRows), method("insertRow", O::SectionInsertRow, 0), method("deleteRow", O::SectionDeleteRow, 1),
    getter("align", O::LegacyReflectGet(I::HTMLTableSectionElement, "align", LR::String)), setter("align", O::LegacyReflectSet(I::HTMLTableSectionElement, "align", LR::String)),
    getter("ch", O::LegacyReflectGet(I::HTMLTableSectionElement, "char", LR::String)), setter("ch", O::LegacyReflectSet(I::HTMLTableSectionElement, "char", LR::String)),
    getter("chOff", O::LegacyReflectGet(I::HTMLTableSectionElement, "charoff", LR::String)), setter("chOff", O::LegacyReflectSet(I::HTMLTableSectionElement, "charoff", LR::String)),
    getter("vAlign", O::LegacyReflectGet(I::HTMLTableSectionElement, "valign", LR::String)), setter("vAlign", O::LegacyReflectSet(I::HTMLTableSectionElement, "valign", LR::String)),
];
const LEGACY_TABLESECTION: [Member; (EMPTY_LEGACY_BASE).len() + LEGACY_TABLESECTION_EXTRA.len()] = with_legacy_members(EMPTY_LEGACY_BASE, LEGACY_TABLESECTION_EXTRA);
const LEGACY_TABLECELL_EXTRA: &[Member] = &[
    getter("align", O::LegacyReflectGet(I::HTMLTableCellElement, "align", LR::String)), setter("align", O::LegacyReflectSet(I::HTMLTableCellElement, "align", LR::String)),
    getter("axis", O::LegacyReflectGet(I::HTMLTableCellElement, "axis", LR::String)), setter("axis", O::LegacyReflectSet(I::HTMLTableCellElement, "axis", LR::String)),
    getter("height", O::LegacyReflectGet(I::HTMLTableCellElement, "height", LR::String)), setter("height", O::LegacyReflectSet(I::HTMLTableCellElement, "height", LR::String)),
    getter("width", O::LegacyReflectGet(I::HTMLTableCellElement, "width", LR::String)), setter("width", O::LegacyReflectSet(I::HTMLTableCellElement, "width", LR::String)),
    getter("ch", O::LegacyReflectGet(I::HTMLTableCellElement, "char", LR::String)), setter("ch", O::LegacyReflectSet(I::HTMLTableCellElement, "char", LR::String)),
    getter("chOff", O::LegacyReflectGet(I::HTMLTableCellElement, "charoff", LR::String)), setter("chOff", O::LegacyReflectSet(I::HTMLTableCellElement, "charoff", LR::String)),
    getter("noWrap", O::LegacyReflectGet(I::HTMLTableCellElement, "nowrap", LR::Boolean)), setter("noWrap", O::LegacyReflectSet(I::HTMLTableCellElement, "nowrap", LR::Boolean)),
    getter("vAlign", O::LegacyReflectGet(I::HTMLTableCellElement, "valign", LR::String)), setter("vAlign", O::LegacyReflectSet(I::HTMLTableCellElement, "valign", LR::String)),
    getter("bgColor", O::LegacyReflectGet(I::HTMLTableCellElement, "bgcolor", LR::LegacyString)), setter("bgColor", O::LegacyReflectSet(I::HTMLTableCellElement, "bgcolor", LR::LegacyString)),
];
const LEGACY_TABLECELL: [Member; (CELL).len() + LEGACY_TABLECELL_EXTRA.len()] = with_legacy_members(CELL, LEGACY_TABLECELL_EXTRA);
const LEGACY_TABLEROW_EXTRA: &[Member] = &[
    getter("rowIndex", O::RowIndex), getter("sectionRowIndex", O::RowSectionIndex), getter("cells", O::RowCells),
    method("insertCell", O::RowInsertCell, 0), method("deleteCell", O::RowDeleteCell, 1),
    getter("align", O::LegacyReflectGet(I::HTMLTableRowElement, "align", LR::String)), setter("align", O::LegacyReflectSet(I::HTMLTableRowElement, "align", LR::String)),
    getter("ch", O::LegacyReflectGet(I::HTMLTableRowElement, "char", LR::String)), setter("ch", O::LegacyReflectSet(I::HTMLTableRowElement, "char", LR::String)),
    getter("chOff", O::LegacyReflectGet(I::HTMLTableRowElement, "charoff", LR::String)), setter("chOff", O::LegacyReflectSet(I::HTMLTableRowElement, "charoff", LR::String)),
    getter("vAlign", O::LegacyReflectGet(I::HTMLTableRowElement, "valign", LR::String)), setter("vAlign", O::LegacyReflectSet(I::HTMLTableRowElement, "valign", LR::String)),
    getter("bgColor", O::LegacyReflectGet(I::HTMLTableRowElement, "bgcolor", LR::LegacyString)), setter("bgColor", O::LegacyReflectSet(I::HTMLTableRowElement, "bgcolor", LR::LegacyString)),
];
const LEGACY_TABLEROW: [Member; (EMPTY_LEGACY_BASE).len() + LEGACY_TABLEROW_EXTRA.len()] = with_legacy_members(EMPTY_LEGACY_BASE, LEGACY_TABLEROW_EXTRA);
const LEGACY_ULIST_EXTRA: &[Member] = &[
    getter("compact", O::LegacyReflectGet(I::HTMLUListElement, "compact", LR::Boolean)), setter("compact", O::LegacyReflectSet(I::HTMLUListElement, "compact", LR::Boolean)),
    getter("type", O::LegacyReflectGet(I::HTMLUListElement, "type", LR::String)), setter("type", O::LegacyReflectSet(I::HTMLUListElement, "type", LR::String)),
];
const LEGACY_ULIST: [Member; (EMPTY_LEGACY_BASE).len() + LEGACY_ULIST_EXTRA.len()] = with_legacy_members(EMPTY_LEGACY_BASE, LEGACY_ULIST_EXTRA);
pub static INTERFACES: &[D] = &[
    D { interface: I::Location, name: "Location", parent: None,
        constructor: None, members: LOCATION },
    D { interface: I::DOMStringList, name: "DOMStringList", parent: None,
        constructor: None, members: DOM_STRING_LIST },
    D { interface: I::XMLSerializer, name: "XMLSerializer", parent: None,
        constructor: Some(O::XMLSerializerConstructor), members: XML_SERIALIZER },
    D { interface: I::DOMParser, name: "DOMParser", parent: None,
        constructor: Some(O::DOMParserConstructor), members: DOM_PARSER },
    D { interface: I::URL, name: "URL", parent: None, constructor: Some(O::URLConstructor), members: URL },
    D { interface: I::URLSearchParams, name: "URLSearchParams", parent: None,
        constructor: Some(O::URLSearchParamsConstructor), members: URL_SEARCH_PARAMS },
    D { interface: I::Blob, name: "Blob", parent: None, constructor: Some(O::BlobConstructor), members: BLOB },
    D { interface: I::File, name: "File", parent: Some(I::Blob), constructor: Some(O::FileConstructor), members: FILE },
    D { interface: I::FormData, name: "FormData", parent: None, constructor: Some(O::FormDataFormConstructor), members: FORM_DATA },
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
    D { interface: I::Observable, name: "Observable", parent: None,
        constructor: Some(O::ObservableConstructor), members: OBSERVABLE },
    D { interface: I::Subscriber, name: "Subscriber", parent: None,
        constructor: None, members: SUBSCRIBER },
    D { interface: I::Animation, name: "Animation", parent: None,
        constructor: None, members: ANIMATION },
    D { interface: I::Clipboard, name: "Clipboard", parent: None,
        constructor: None, members: CLIPBOARD },
    D { interface: I::CSSStyleDeclaration, name: "CSSStyleDeclaration", parent: None,
        constructor: None, members: &[
            getter("cssText", O::Cssom(crate::cssom::CssomOp::CssText)),
            setter("cssText", O::Cssom(crate::cssom::CssomOp::SetCssText)),
            getter("length", O::Cssom(crate::cssom::CssomOp::Length)),
            method("item", O::Cssom(crate::cssom::CssomOp::Item), 1),
            method("getPropertyValue", O::Cssom(crate::cssom::CssomOp::GetPropertyValue), 1),
            method("getPropertyPriority", O::Cssom(crate::cssom::CssomOp::GetPropertyPriority), 1),
            method("setProperty", O::Cssom(crate::cssom::CssomOp::SetProperty), 2),
            method("removeProperty", O::Cssom(crate::cssom::CssomOp::RemoveProperty), 1),
            getter("parentRule", O::Cssom(crate::cssom::CssomOp::ParentRule)),
        ] },
    D { interface: I::CSSStyleProperties, name: "CSSStyleProperties", parent: Some(I::CSSStyleDeclaration),
        constructor: None, members: &[
            getter("cssFloat", O::Cssom(crate::cssom::CssomOp::CssFloat)),
            setter("cssFloat", O::Cssom(crate::cssom::CssomOp::SetCssFloat)),
        ] },
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
    D { interface: I::HTMLElement, name: "HTMLElement", parent: Some(I::Element), constructor: Some(O::HTMLConstructor(I::HTMLElement)), members: &HTML_ELEMENT },
    D { interface: I::SVGElement, name: "SVGElement", parent: Some(I::Element), constructor: None,
        members: &[getter("dataset", O::ElementDataset), method("focus", O::ElementFocus, 0), method("blur", O::ElementBlur, 0)] },
    D { interface: I::MathMLElement, name: "MathMLElement", parent: Some(I::Element), constructor: None,
        members: &[getter("dataset", O::ElementDataset), method("focus", O::ElementFocus, 0), method("blur", O::ElementBlur, 0)] },
    D { interface: I::HTMLDirectoryElement, name: "HTMLDirectoryElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLDirectoryElement)), members: &[
        getter("compact", O::DirectoryCompact), setter("compact", O::DirectoryCompactSet)
    ] },
    D { interface: I::HTMLFontElement, name: "HTMLFontElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLFontElement)), members: &[
        getter("color", O::FontColor), setter("color", O::FontColorSet),
        getter("face", O::FontFace), setter("face", O::FontFaceSet),
        getter("size", O::FontSize), setter("size", O::FontSizeSet)
    ] },
    D { interface: I::HTMLFrameElement, name: "HTMLFrameElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLFrameElement)), members: &[
        getter("name", O::FrameName), setter("name", O::FrameNameSet),
        getter("scrolling", O::FrameScrolling), setter("scrolling", O::FrameScrollingSet),
        getter("src", O::FrameSrc), setter("src", O::FrameSrcSet),
        getter("frameBorder", O::FrameBorder), setter("frameBorder", O::FrameBorderSet),
        getter("longDesc", O::FrameLongDesc), setter("longDesc", O::FrameLongDescSet),
        getter("noResize", O::FrameNoResize), setter("noResize", O::FrameNoResizeSet),
        getter("marginHeight", O::FrameMarginHeight), setter("marginHeight", O::FrameMarginHeightSet),
        getter("marginWidth", O::FrameMarginWidth), setter("marginWidth", O::FrameMarginWidthSet)
    ] },
    D { interface: I::HTMLFrameSetElement, name: "HTMLFrameSetElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLFrameSetElement)), members: &[
        getter("cols", O::FrameSetCols), setter("cols", O::FrameSetColsSet),
        getter("rows", O::FrameSetRows), setter("rows", O::FrameSetRowsSet)
    ] },
    D { interface: I::HTMLUnknownElement, name: "HTMLUnknownElement", parent: Some(I::HTMLElement), constructor: None, members: &[] },
    D { interface: I::HTMLHtmlElement, name: "HTMLHtmlElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLHtmlElement)), members: &LEGACY_HTML },
    D { interface: I::HTMLHeadElement, name: "HTMLHeadElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLHeadElement)), members: &[] },
    D { interface: I::HTMLTitleElement, name: "HTMLTitleElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLTitleElement)), members: &[] },
    D { interface: I::HTMLBaseElement, name: "HTMLBaseElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLBaseElement)), members: &[
        getter("href", O::LegacyReflectGet(I::HTMLBaseElement, "href", LR::UsvUrl)),
        setter("href", O::LegacyReflectSet(I::HTMLBaseElement, "href", LR::UsvUrl)),
        getter("target", O::LegacyReflectGet(I::HTMLBaseElement, "target", LR::String)),
        setter("target", O::LegacyReflectSet(I::HTMLBaseElement, "target", LR::String)),
    ] },
    D { interface: I::HTMLLinkElement, name: "HTMLLinkElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLLinkElement)), members: &reflection_extra::LINK },
    D { interface: I::HTMLMetaElement, name: "HTMLMetaElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLMetaElement)), members: &reflection_extra::META_ELEMENT },
    D { interface: I::HTMLStyleElement, name: "HTMLStyleElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLStyleElement)), members: &reflection_extra::STYLE },
    D { interface: I::HTMLBodyElement, name: "HTMLBodyElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLBodyElement)), members: &LEGACY_BODY },
    D { interface: I::HTMLHeadingElement, name: "HTMLHeadingElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLHeadingElement)), members: &LEGACY_HEADING },
    D { interface: I::HTMLParagraphElement, name: "HTMLParagraphElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLParagraphElement)), members: &LEGACY_PARAGRAPH },
    D { interface: I::HTMLHRElement, name: "HTMLHRElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLHRElement)), members: &LEGACY_HR },
    D { interface: I::HTMLPreElement, name: "HTMLPreElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLPreElement)), members: &LEGACY_PRE },
    D { interface: I::HTMLQuoteElement, name: "HTMLQuoteElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLQuoteElement)), members: reflection_extra::QUOTE },
    D { interface: I::HTMLOListElement, name: "HTMLOListElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLOListElement)), members: &reflection_extra::OLIST },
    D { interface: I::HTMLUListElement, name: "HTMLUListElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLUListElement)), members: &LEGACY_ULIST },
    D { interface: I::HTMLLIElement, name: "HTMLLIElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLLIElement)), members: &LEGACY_LI },
    D { interface: I::HTMLDListElement, name: "HTMLDListElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLDListElement)), members: &LEGACY_DLIST },
    D { interface: I::HTMLDivElement, name: "HTMLDivElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLDivElement)), members: &LEGACY_DIV },
    D { interface: I::HTMLSpanElement, name: "HTMLSpanElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLSpanElement)), members: &[] },
    D { interface: I::HTMLAnchorElement, name: "HTMLAnchorElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLAnchorElement)), members: &reflection_extra::ANCHOR },
    D { interface: I::HTMLImageElement, name: "HTMLImageElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLImageElement)), members: &reflection_extra::IMAGE_ELEMENT },
    D { interface: I::HTMLBRElement, name: "HTMLBRElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLBRElement)), members: &LEGACY_BR },
    D { interface: I::HTMLModElement, name: "HTMLModElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLModElement)), members: reflection_extra::MOD },
    D { interface: I::HTMLTimeElement, name: "HTMLTimeElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLTimeElement)), members: reflection_extra::TIME },
    D { interface: I::HTMLDataElement, name: "HTMLDataElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLDataElement)), members: reflection_extra::DATA },
    D { interface: I::HTMLTableElement, name: "HTMLTableElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLTableElement)), members: &LEGACY_TABLE },
    D { interface: I::HTMLTableCaptionElement, name: "HTMLTableCaptionElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLTableCaptionElement)), members: &LEGACY_TABLECAPTION },
    D { interface: I::HTMLTableSectionElement, name: "HTMLTableSectionElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLTableSectionElement)), members: &LEGACY_TABLESECTION },
    D { interface: I::HTMLTableRowElement, name: "HTMLTableRowElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLTableRowElement)), members: &LEGACY_TABLEROW },
    D { interface: I::HTMLTableCellElement, name: "HTMLTableCellElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLTableCellElement)), members: &reflection_extra::TABLECELL },
    D { interface: I::HTMLTableColElement, name: "HTMLTableColElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLTableColElement)), members: &reflection_extra::TABLECOL },
    D { interface: I::HTMLFormElement, name: "HTMLFormElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLFormElement)), members: FORM },
    D { interface: I::HTMLInputElement, name: "HTMLInputElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLInputElement)), members: &LEGACY_INPUT },
    D { interface: I::HTMLButtonElement, name: "HTMLButtonElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLButtonElement)), members: BUTTON },
    D { interface: I::HTMLSelectElement, name: "HTMLSelectElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLSelectElement)), members: SELECT },
    D { interface: I::HTMLOptionElement, name: "HTMLOptionElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLOptionElement)), members: OPTION },
    D { interface: I::HTMLOptGroupElement, name: "HTMLOptGroupElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLOptGroupElement)), members: OPTGROUP },
    D { interface: I::HTMLTextAreaElement, name: "HTMLTextAreaElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLTextAreaElement)), members: TEXTAREA },
    D { interface: I::HTMLLabelElement, name: "HTMLLabelElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLLabelElement)), members: LABEL },
    D { interface: I::HTMLFieldSetElement, name: "HTMLFieldSetElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLFieldSetElement)), members: FIELDSET },
    D { interface: I::HTMLLegendElement, name: "HTMLLegendElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLLegendElement)), members: &LEGACY_LEGEND },
    D { interface: I::HTMLDataListElement, name: "HTMLDataListElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLDataListElement)), members: &[] },
    D { interface: I::HTMLOutputElement, name: "HTMLOutputElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLOutputElement)), members: OUTPUT },
    D { interface: I::HTMLProgressElement, name: "HTMLProgressElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLProgressElement)), members: PROGRESS },
    D { interface: I::HTMLMeterElement, name: "HTMLMeterElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLMeterElement)), members: METER },
    D { interface: I::HTMLMediaElement, name: "HTMLMediaElement", parent: Some(I::HTMLElement), constructor: None, members: reflection_extra::MEDIA },
    D { interface: I::HTMLVideoElement, name: "HTMLVideoElement", parent: Some(I::HTMLMediaElement), constructor: Some(O::HTMLConstructor(I::HTMLVideoElement)), members: reflection_extra::VIDEO },
    D { interface: I::HTMLAudioElement, name: "HTMLAudioElement", parent: Some(I::HTMLMediaElement), constructor: Some(O::HTMLConstructor(I::HTMLAudioElement)), members: &[] },
    D { interface: I::HTMLSourceElement, name: "HTMLSourceElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLSourceElement)), members: &reflection_extra::SOURCE_ELEMENT },
    D { interface: I::HTMLTrackElement, name: "HTMLTrackElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLTrackElement)), members: reflection_extra::TRACK },
    D { interface: I::HTMLCanvasElement, name: "HTMLCanvasElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLCanvasElement)), members: &[] },
    D { interface: I::HTMLScriptElement, name: "HTMLScriptElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLScriptElement)), members: &reflection_extra::SCRIPT },
    D { interface: I::HTMLTemplateElement, name: "HTMLTemplateElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLTemplateElement)), members: &[
        getter("content", O::TemplateContent),
        getter("htmlFor", O::LegacyReflectGet(I::HTMLTemplateElement, "for", LR::String)),
        setter("htmlFor", O::LegacyReflectSet(I::HTMLTemplateElement, "for", LR::String)),
        // DOM-18 follow-up: declarative shadow DOM's four reflected IDL
        // attributes on <template> (HTML SS4.12.4). shadowRootMode is a
        // "limited to only known values" DOMString (getter-only limiting,
        // default ""); the other three are plain boolean-attribute
        // reflections, same shape as every other Boolean LegacyReflectGet/
        // Set pair above.
        getter("shadowRootMode", O::LegacyReflectGet(I::HTMLTemplateElement, "shadowrootmode", LR::LimitedKnownValues(&["open", "closed"]))),
        setter("shadowRootMode", O::LegacyReflectSet(I::HTMLTemplateElement, "shadowrootmode", LR::LimitedKnownValues(&["open", "closed"]))),
        getter("shadowRootDelegatesFocus", O::LegacyReflectGet(I::HTMLTemplateElement, "shadowrootdelegatesfocus", LR::Boolean)),
        setter("shadowRootDelegatesFocus", O::LegacyReflectSet(I::HTMLTemplateElement, "shadowrootdelegatesfocus", LR::Boolean)),
        getter("shadowRootClonable", O::LegacyReflectGet(I::HTMLTemplateElement, "shadowrootclonable", LR::Boolean)),
        setter("shadowRootClonable", O::LegacyReflectSet(I::HTMLTemplateElement, "shadowrootclonable", LR::Boolean)),
        getter("shadowRootSerializable", O::LegacyReflectGet(I::HTMLTemplateElement, "shadowrootserializable", LR::Boolean)),
        setter("shadowRootSerializable", O::LegacyReflectSet(I::HTMLTemplateElement, "shadowrootserializable", LR::Boolean)),
    ] },
    D { interface: I::HTMLSlotElement, name: "HTMLSlotElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLSlotElement)), members: SLOT },
    D { interface: I::HTMLIFrameElement, name: "HTMLIFrameElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLIFrameElement)), members: &reflection_extra::IFRAME_ELEMENT },
    D { interface: I::HTMLEmbedElement, name: "HTMLEmbedElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLEmbedElement)), members: &reflection_extra::EMBED },
    D { interface: I::HTMLObjectElement, name: "HTMLObjectElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLObjectElement)), members: &reflection_extra::OBJECT },
    D { interface: I::HTMLParamElement, name: "HTMLParamElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLParamElement)), members: &LEGACY_PARAM },
    D { interface: I::HTMLDetailsElement, name: "HTMLDetailsElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLDetailsElement)), members: reflection_extra::DETAILS },
    D { interface: I::HTMLDialogElement, name: "HTMLDialogElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLDialogElement)), members: reflection_extra::DIALOG },
    D { interface: I::HTMLMapElement, name: "HTMLMapElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLMapElement)), members: reflection_extra::MAP },
    D { interface: I::HTMLAreaElement, name: "HTMLAreaElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLAreaElement)), members: &reflection_extra::AREA },
    D { interface: I::HTMLMarqueeElement, name: "HTMLMarqueeElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLMarqueeElement)), members: reflection_extra::MARQUEE },
    D { interface: I::HTMLPictureElement, name: "HTMLPictureElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLPictureElement)), members: &[] },
    D { interface: I::HTMLMenuElement, name: "HTMLMenuElement", parent: Some(I::HTMLElement), constructor: Some(O::HTMLConstructor(I::HTMLMenuElement)), members: &LEGACY_MENU },
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
        constructor: Some(O::ProcessingInstructionConstructor),
        members: &[getter("target", O::ProcessingInstructionTarget),
            method("hasAttributes", O::PiHasAttributes, 0),
            method("getAttributeNames", O::PiGetAttributeNames, 0),
            method("getAttribute", O::PiGetAttribute, 1),
            method("setAttribute", O::PiSetAttribute, 2),
            method("removeAttribute", O::PiRemoveAttribute, 1),
            method("toggleAttribute", O::PiToggleAttribute, 1),
            method("hasAttribute", O::PiHasAttribute, 1),
        ],
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
    D { interface: I::ElementInternals, name: "ElementInternals", parent: None, constructor: None, members: ELEMENT_INTERNALS },
    D { interface: I::CustomStateSet, name: "CustomStateSet", parent: None, constructor: None, members: CUSTOM_STATE_SET },
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
    D { interface: I::HashChangeEvent, name: "HashChangeEvent", parent: Some(I::Event),
        constructor: Some(O::HashChangeEventConstructor), members: &[
            getter("oldURL", O::HashChangeOldURL), getter("newURL", O::HashChangeNewURL)] },
    D { interface: I::PopStateEvent, name: "PopStateEvent", parent: Some(I::Event),
        constructor: Some(O::PopStateEventConstructor), members: &[
            getter("state", O::PopStateState), getter("hasUAVisualTransition", O::PopStateHasUAVisualTransition)] },
    D { interface: I::PageTransitionEvent, name: "PageTransitionEvent", parent: Some(I::Event),
        constructor: Some(O::PageTransitionEventConstructor), members: &[getter("persisted", O::PageTransitionPersisted)] },
    D { interface: I::ErrorEvent, name: "ErrorEvent", parent: Some(I::Event),
        constructor: Some(O::ErrorEventConstructor), members: &[
            getter("message", O::ErrorEventGet("message")), getter("filename", O::ErrorEventGet("filename")),
            getter("lineno", O::ErrorEventGet("lineno")), getter("colno", O::ErrorEventGet("colno")),
            getter("error", O::ErrorEventGet("error"))] },
    D { interface: I::PromiseRejectionEvent, name: "PromiseRejectionEvent", parent: Some(I::Event),
        constructor: Some(O::PromiseRejectionEventConstructor), members: &[
            getter("promise", O::PromiseRejectionEventGet("promise")), getter("reason", O::PromiseRejectionEventGet("reason"))] },
    D { interface: I::BeforeUnloadEvent, name: "BeforeUnloadEvent", parent: Some(I::Event),
        constructor: None, members: &[getter("returnValue", O::BeforeUnloadReturnValue),
            setter("returnValue", O::BeforeUnloadSetReturnValue)] },
    D { interface: I::ProgressEvent, name: "ProgressEvent", parent: Some(I::Event),
        constructor: Some(O::ProgressEventConstructor), members: PROGRESS_EVENT },
    D { interface: I::CompositionEvent, name: "CompositionEvent", parent: Some(I::UIEvent),
        constructor: Some(O::CompositionEventConstructor), members: COMPOSITION_EVENT },
    // Legacy `document.createEvent()`-only aliases: real DOM/HTML interfaces this engine does
    // not otherwise implement (device orientation, drag-and-drop, web storage, legacy IME text
    // events). No constructor, matching BeforeUnloadEvent above -- `new X()` is unsupported but
    // `window.X.prototype` must exist so createEvent(name)'s alias-identity checks pass.
    D { interface: I::DeviceMotionEvent, name: "DeviceMotionEvent", parent: Some(I::Event),
        constructor: None, members: &[] },
    D { interface: I::DeviceOrientationEvent, name: "DeviceOrientationEvent", parent: Some(I::Event),
        constructor: None, members: &[] },
    D { interface: I::DragEvent, name: "DragEvent", parent: Some(I::MouseEvent),
        constructor: None, members: &[] },
    D { interface: I::StorageEvent, name: "StorageEvent", parent: Some(I::Event),
        constructor: None, members: &[] },
    D { interface: I::TextEvent, name: "TextEvent", parent: Some(I::UIEvent),
        constructor: None, members: &[] },
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
        "marquee" => I::HTMLMarqueeElement,
        "article" | "aside" | "b" | "bdi" | "bdo" | "big" | "center" | "cite" | "code" | "dd" | "dfn" | "dt" | "em" | "figcaption" | "figure" | "footer" | "header" | "hgroup" | "i" | "kbd" | "main" | "mark" | "nav" | "nobr" | "noembed" | "noframes" | "noscript" | "plaintext" | "rb" | "rp" | "rt" | "rtc" | "ruby" | "s" | "samp" | "search" | "section" | "small" | "strike" | "strong" | "sub" | "sup" | "tt" | "u" | "var" | "wbr" | "address" | "abbr" | "acronym" | "basefont" => I::HTMLElement,
        name if amazing_dom::Dom::valid_custom_element_name(name) => I::HTMLElement,
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
    host.install_static_method(I::Observable, "from", O::ObservableFrom, 1)?;
    host.install_static_method(I::Document, "parseHTMLUnsafe", O::DocumentParseHTMLUnsafe, 1)?;
    host.install_static_method(I::URL, "canParse", O::URLCanParse, 1)?;
    host.install_static_method(I::URL, "parse", O::URLParse, 1)?;
    host.install_static_method(I::URL, "createObjectURL", O::URLCreateObjectURL, 1)?;
    host.install_static_method(I::URL, "revokeObjectURL", O::URLRevokeObjectURL, 1)?;
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
    host.define_legacy_platform_object(InterfaceBrand::SELECTED_OPTIONS, LegacyPlatformHooks {
        supported_index_count: O::SelectedOptionsSupportedIndexCount,
        supported_names: None,
        indexed_getter: O::SelectedOptionsIndexedGetter, indexed_setter: None,
        named_getter: Some(O::SelectedOptionsNamedGetter), named_setter: None, named_deleter: None,
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
    host.define_set_like_iterator(I::CustomStateSet)?;
    host.define_array_values_iterator(I::HTMLCollection)?;
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
    if host.install_cssom()? == OptionalHookInstall::NotSupported {
        report.not_installed.push(OptionalCapability::CssomGlobal);
    }
    Ok(report)
}
