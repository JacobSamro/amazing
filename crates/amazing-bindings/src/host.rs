//! Runtime-owned values and installation hooks; this crate never imports JS.
use amazing_dom::{Dom, DomError, NodeIdx};
pub use amazing_dom::events::CallbackId;
/// Opaque driver-owned strong reference to a JS value. Native state stores
/// only this token; the driver maps it to a value outside a DOM borrow.
pub type RootId = CallbackId;

/// Identity of an arena object. A reused slot must receive a new generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PlatformKey {
    pub arena: u64,
    pub slot: u64,
    pub generation: u64,
}

/// Private runtime brand for a family of platform objects. Inheritance within
/// a family is checked by bindings against native state, not JS properties.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct InterfaceBrand(pub u64);

impl InterfaceBrand {
    pub const NODE: Self = Self(1);
    pub const RANGE: Self = Self(2);
    pub const TRAVERSAL: Self = Self(3);
    pub const EVENT: Self = Self(4);
    pub const MUTATION_OBSERVER: Self = Self(5);
    pub const MUTATION_RECORD: Self = Self(6);
    pub const ATTRIBUTE: Self = Self(7);
    pub const COLLECTION: Self = Self(8);
    pub const DOM_EXCEPTION: Self = Self(9);
    pub const DOM_IMPLEMENTATION: Self = Self(10);
    pub const STATIC_RANGE: Self = Self(11);
    pub const NODE_LIST: Self = Self(12);
    pub const NODE_ITERATOR: Self = Self(13);
    pub const TREE_WALKER: Self = Self(14);
    pub const EVENT_TARGET: Self = Self(15);
    pub const STATIC_NODE_LIST: Self = Self(16);
    pub const NAMED_NODE_MAP: Self = Self(17);
    pub const DOM_TOKEN_LIST: Self = Self(18);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Interface {
    EventTarget,
    Node,
    CDATASection,
    Attr,
    NamedNodeMap,
    DOMTokenList,
    Document,
    DocumentFragment,
    DocumentType,
    Element,
    HTMLElement,
    HTMLHtmlElement,
    HTMLHeadElement,
    HTMLTitleElement,
    HTMLBaseElement,
    HTMLLinkElement,
    HTMLMetaElement,
    HTMLStyleElement,
    HTMLBodyElement,
    HTMLHeadingElement,
    HTMLParagraphElement,
    HTMLHRElement,
    HTMLPreElement,
    HTMLQuoteElement,
    HTMLOListElement,
    HTMLUListElement,
    HTMLLIElement,
    HTMLDListElement,
    HTMLDivElement,
    HTMLAnchorElement,
    HTMLImageElement,
    HTMLBRElement,
    HTMLModElement,
    HTMLTimeElement,
    HTMLDataElement,
    HTMLTableElement,
    HTMLTableCaptionElement,
    HTMLTableSectionElement,
    HTMLTableRowElement,
    HTMLTableCellElement,
    HTMLTableColElement,
    HTMLFormElement,
    HTMLInputElement,
    HTMLButtonElement,
    HTMLSelectElement,
    HTMLOptionElement,
    HTMLOptGroupElement,
    HTMLTextAreaElement,
    HTMLLabelElement,
    HTMLFieldSetElement,
    HTMLLegendElement,
    HTMLDataListElement,
    HTMLOutputElement,
    HTMLProgressElement,
    HTMLMeterElement,
    HTMLVideoElement,
    HTMLMediaElement,
    HTMLAudioElement,
    HTMLSourceElement,
    HTMLTrackElement,
    HTMLCanvasElement,
    HTMLScriptElement,
    HTMLTemplateElement,
    HTMLSlotElement,
    HTMLIFrameElement,
    HTMLEmbedElement,
    HTMLObjectElement,
    HTMLParamElement,
    HTMLDetailsElement,
    HTMLDialogElement,
    HTMLMapElement,
    HTMLAreaElement,
    HTMLPictureElement,
    HTMLMenuElement,
    HTMLUnknownElement,
    HTMLCollection,
    CharacterData,
    Text,
    Comment,
    ProcessingInstruction,
    AbstractRange,
    StaticRange,
    Range,
    DOMImplementation,
    NodeFilter,
    NodeList,
    NodeIterator,
    TreeWalker,
    Event,
    CustomEvent,
    MutationObserver,
    MutationRecord,
    DOMException,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    TextConstructor,
    CommentConstructor,
    FragmentConstructor,
    DocumentConstructor,
    CreateText,
    CreateCDATASection,
    CreateAttribute,
    CreateAttributeNS,
    CreateProcessingInstruction,
    ElementAttributes,
    AttrNamespaceURI,
    AttrPrefix,
    AttrLocalName,
    AttrName,
    AttrValue,
    AttrSetValue,
    AttrOwnerElement,
    AttrSpecified,
    ElementGetAttribute,
    ElementGetAttributeNS,
    ElementSetAttribute,
    ElementSetAttributeNS,
    ElementRemoveAttribute,
    ElementRemoveAttributeNS,
    ElementGetAttributeNode,
    ElementGetAttributeNodeNS,
    ElementSetAttributeNode,
    ElementSetAttributeNodeNS,
    ElementRemoveAttributeNode,
    NamedNodeMapLength,
    NamedNodeMapItem,
    NamedNodeMapGetNamedItem,
    NamedNodeMapGetNamedItemNS,
    NamedNodeMapSetNamedItem,
    NamedNodeMapSetNamedItemNS,
    NamedNodeMapRemoveNamedItem,
    NamedNodeMapRemoveNamedItemNS,
    ElementClassList,
    ElementClassName,
    ElementSetClassName,
    ElementId,
    ElementSetId,
    HTMLTitle,
    HTMLSetTitle,
    HTMLLang,
    HTMLSetLang,
    HTMLDir,
    HTMLSetDir,
    HTMLHidden,
    HTMLSetHidden,
    HTMLTabIndex,
    HTMLSetTabIndex,
    HTMLAccessKey,
    HTMLSetAccessKey,
    HTMLTranslate,
    HTMLSetTranslate,
    TokenListLength,
    TokenListItem,
    TokenListSupportedIndices,
    TokenListIndexedGetter,
    TokenListContains,
    TokenListAdd,
    TokenListRemove,
    TokenListToggle,
    TokenListReplace,
    TokenListSupports,
    TokenListValue,
    TokenListSetValue,
    CreateComment,
    CreateFragment,
    Append,
    Prepend,
    ReplaceChildren,
    Before,
    After,
    ReplaceWith,
    Remove,
    AppendChild,
    InsertBefore,
    RemoveChild,
    ReplaceChild,
    CloneNode,
    NodeType,
    NodeName,
    NodeValue,
    SetNodeValue,
    TextContent,
    SetTextContent,
    ParentNode,
    ParentElement,
    OwnerDocument,
    FirstChild,
    LastChild,
    PreviousSibling,
    NextSibling,
    ChildNodes,
    HasChildNodes,
    IsConnected,
    Contains,
    Data,
    SetData,
    Length,
    RangeConstructor,
    StaticRangeConstructor,
    CreateRange,
    RangeStartContainer,
    RangeStartOffset,
    RangeEndContainer,
    RangeEndOffset,
    RangeCollapsed,
    RangeCommonAncestorContainer,
    RangeSetStart,
    RangeSetEnd,
    RangeSetStartBefore,
    RangeSetStartAfter,
    RangeSetEndBefore,
    RangeSetEndAfter,
    RangeCollapse,
    RangeSelectNode,
    RangeSelectNodeContents,
    RangeDeleteContents,
    RangeExtractContents,
    RangeCloneContents,
    RangeInsertNode,
    RangeSurroundContents,
    RangeCloneRange,
    RangeDetach,
    RangeIntersectsNode,
    RangeCompareBoundaryPoints,
    RangeIsPointInRange,
    RangeComparePoint,
    RangeStringifier,
    DocumentImplementation,
    ImplementationCreateDocumentType,
    ImplementationCreateDocument,
    ImplementationCreateHTMLDocument,
    ImplementationHasFeature,
    CreateElement,
    CreateElementNS,
    NodeListLength,
    NodeListItem,
    NodeListSupportedIndices,
    NodeListIndexedGetter,
    ElementChildren,
    DocumentChildren,
    FragmentChildren,
    ElementGetElementsByTagName,
    ElementGetElementsByTagNameNS,
    ElementGetElementsByClassName,
    DocumentGetElementsByTagName,
    DocumentGetElementsByTagNameNS,
    DocumentGetElementsByClassName,
    HTMLCollectionLength,
    HTMLCollectionItem,
    HTMLCollectionNamedItem,
    HTMLCollectionSupportedIndices,
    HTMLCollectionSupportedNames,
    HTMLCollectionIndexedGetter,
    HTMLCollectionNamedGetter,
    CreateNodeIterator,
    CreateTreeWalker,
    TraversalRoot,
    TraversalWhatToShow,
    TraversalFilter,
    IteratorReferenceNode,
    IteratorPointerBeforeReferenceNode,
    IteratorNextNode,
    IteratorPreviousNode,
    IteratorDetach,
    WalkerCurrentNode,
    WalkerSetCurrentNode,
    WalkerParentNode,
    WalkerFirstChild,
    WalkerLastChild,
    WalkerPreviousSibling,
    WalkerNextSibling,
    WalkerPreviousNode,
    WalkerNextNode,
    EventTargetConstructor,
    AddEventListener,
    RemoveEventListener,
    DispatchEvent,
    EventConstructor,
    CustomEventConstructor,
    CustomEventDetail,
    CustomEventInit,
    CreateEvent,
    EventType,
    EventTargetValue,
    EventCurrentTarget,
    EventPhase,
    EventBubbles,
    EventCancelable,
    EventComposed,
    EventDefaultPrevented,
    EventIsTrusted,
    EventTimeStamp,
    EventStopPropagation,
    EventStopImmediatePropagation,
    EventPreventDefault,
    EventComposedPath,
    EventInitEvent,
    EventCancelBubble,
    EventSetCancelBubble,
    EventReturnValue,
    EventSetReturnValue,
    MutationObserverConstructor,
    MutationObserve,
    MutationDisconnect,
    MutationTakeRecords,
    MutationRecordType,
    MutationRecordTarget,
    MutationRecordAddedNodes,
    MutationRecordRemovedNodes,
    MutationRecordPreviousSibling,
    MutationRecordNextSibling,
    MutationRecordAttributeName,
    MutationRecordAttributeNamespace,
    MutationRecordOldValue,
    DOMExceptionConstructor,
    DOMExceptionName,
    DOMExceptionMessage,
    DOMExceptionCode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemberKind {
    Method { length: u8 },
    Getter,
    Setter,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Member {
    pub name: &'static str,
    pub operation: Operation,
    pub kind: MemberKind,
}

/// Runtime registers standard descriptors/prototypes and forwards native calls
/// to `invoke`. None means an illegal constructor, not a missing global.
#[derive(Clone, Copy, Debug)]
pub struct InterfaceDefinition {
    pub interface: Interface,
    pub name: &'static str,
    pub parent: Option<Interface>,
    pub constructor: Option<Operation>,
    pub members: &'static [Member],
}

/// WebIDL call and construct behavior for a native function.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostCallPolicy {
    CallOnly,
    ConstructOnly,
    CallAndConstruct,
}

/// Runtime-owned internal methods for WebIDL legacy platform objects. The
/// runtime asks bindings for the currently supported keys and values during
/// [[Get]], [[GetOwnProperty]] and [[OwnPropertyKeys]], after ordinary WebIDL
/// visibility rules. No indexed or named properties are preinstalled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LegacyPlatformHooks {
    pub supported_indices: Operation,
    pub supported_names: Option<Operation>,
    pub indexed_getter: Operation,
    pub named_getter: Option<Operation>,
    pub unenumerable_names: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IterableKind { Values }

/// The binding layer declares this trait. The driver implements it using the
/// runtime's backend-neutral capabilities and its same parsed DOM arena.
///
/// `Operation` is an opaque native callback token, not an interpreter body or
/// tree-walker type. The driver maps it to a per-interpreter, runtime-owned
/// `HostFunctionId` and dispatches it to `invoke`. `HostCallPolicy` expresses
/// call-only, construct-only and call-and-construct WebIDL functions. The
/// binding layer assembles prototypes, descriptors, globals and constants
/// from these plain capabilities. `Value` stays runtime-owned. Existing
/// conversion, brand and arena methods below remain separate from installation.
/// js-runtime accepted the capability vocabulary. `install_interface` and
/// `install_node_constant` remain the active installation contract until the
/// driver adapter and installer are ready.
/// `string(&[u16]) -> Result<Value, Error>` and
/// `to_dom_string() -> Result<Vec<u16>, Error>` are UTF-16 boundary contracts.
/// Neither side may use replacement characters for lone surrogates. Until the
/// runtime's opaque UTF-16 `JsString` migration, `string` must reject an
/// unrepresentable value with an explicit TypeError.
///
/// The driver supplies access to its same
/// parsed arena. No user code may run inside `with_dom`; conversions and value
/// wrapping happen outside its borrow. Error values preserve JS exceptions.
///
/// Node brands and wrapper caches must be engine-private (not writable JS
/// properties). `wrap_node` is canonical per arena/realm, including document.
/// Legacy DOM factories and query results must use that same wrapper cache.
pub trait BindingHost: Sized {
    type Value: Clone;
    type Error;

    fn global_object(&mut self) -> Result<Self::Value, Self::Error>;
    fn create_object(
        &mut self,
        prototype: Option<&Self::Value>,
    ) -> Result<Self::Value, Self::Error>;
    fn create_native_function(
        &mut self,
        name: &'static str,
        length: u8,
        operation: Operation,
        policy: HostCallPolicy,
    ) -> Result<Self::Value, Self::Error>;
    fn define_data_property(
        &mut self,
        object: &Self::Value,
        name: &'static str,
        value: Self::Value,
        writable: bool,
        enumerable: bool,
        configurable: bool,
    ) -> Result<(), Self::Error>;
    fn define_accessor_property(
        &mut self,
        object: &Self::Value,
        name: &'static str,
        getter: Option<Self::Value>,
        setter: Option<Self::Value>,
        enumerable: bool,
        configurable: bool,
    ) -> Result<(), Self::Error>;
    fn define_legacy_platform_object(
        &mut self, brand: InterfaceBrand, hooks: LegacyPlatformHooks,
    ) -> Result<(), Self::Error>;
    /// Installs @@iterator, keys, values, entries and forEach on the interface
    /// prototype with runtime-owned iterator state and live indexed reads.
    fn define_iterable(&mut self, interface: Interface, kind: IterableKind) -> Result<(), Self::Error>;
    fn get_property(
        &mut self,
        object: &Self::Value,
        name: &str,
    ) -> Result<Self::Value, Self::Error>;
    fn set_property(
        &mut self,
        object: &Self::Value,
        name: &str,
        value: Self::Value,
    ) -> Result<(), Self::Error>;

    /// Canonical private wrapper in this realm for this key and brand.
    fn wrap_platform_object(
        &mut self,
        key: PlatformKey,
        brand: InterfaceBrand,
    ) -> Result<Self::Value, Self::Error>;
    /// Returns None for an incompatible brand, stale generation or other realm.
    fn platform_object(&self, value: &Self::Value, brand: InterfaceBrand) -> Option<PlatformKey>;
    /// Nodes share one private brand; their exact interface is read from Dom.
    fn node_platform_key(&self, node: NodeIdx) -> PlatformKey;
    fn node_from_platform_key(&self, key: PlatformKey) -> Option<NodeIdx>;

    fn is_callable(&self, value: &Self::Value) -> bool;
    /// Invokes user code with explicit this. Never called inside `with_dom`.
    fn call(
        &mut self,
        function: &Self::Value,
        this: &Self::Value,
        arguments: &[Self::Value],
    ) -> Result<Self::Value, Self::Error>;
    fn same_value(&self, left: &Self::Value, right: &Self::Value) -> bool;
    /// Strong driver-owned value root for callbacks and CustomEvent detail.
    /// The future collector must trace these values from their native owners.
    /// Release listeners on removal or after `once`, and traversal callbacks
    /// when their slots end. An observer callback survives `disconnect` and
    /// releases only when its slot is freed. CustomEvent detail releases when
    /// replaced or its Event slot ends. Until GC exists, a callback capturing
    /// its own target wrapper can form an Rc cycle and leak.
    fn retain_value(&mut self, value: Self::Value) -> RootId;
    fn root_value(&self, root: RootId) -> Option<Self::Value>;
    fn release_value(&mut self, root: RootId);
    fn create_array(&mut self, values: Vec<Self::Value>) -> Result<Self::Value, Self::Error>;
    fn to_number(&mut self, value: &Self::Value) -> Result<f64, Self::Error>;
    fn throw_value(&mut self, value: Self::Value) -> Self::Error;

    fn install_interface(
        &mut self,
        definition: &'static InterfaceDefinition,
    ) -> Result<(), Self::Error>;
    /// Standard Node constants are exposed on Node and its prototype.
    fn install_node_constant(&mut self, name: &'static str, value: u16) -> Result<(), Self::Error>;
    /// Constant on both an interface object and its prototype.
    fn install_interface_constant(
        &mut self,
        interface: Interface,
        name: &'static str,
        value: u32,
    ) -> Result<(), Self::Error>;
    fn current_document(&self) -> NodeIdx;
    fn with_dom<R>(
        &mut self,
        callback: impl FnOnce(&mut Dom) -> Result<R, DomError>,
    ) -> Result<R, Self::Error>;
    /// Non-node values return None without invoking user code.
    fn node_brand(&self, value: &Self::Value) -> Option<NodeIdx> {
        self.platform_object(value, InterfaceBrand::NODE)
            .and_then(|key| self.node_from_platform_key(key))
    }
    fn wrap_node(&mut self, node: NodeIdx) -> Result<Self::Value, Self::Error> {
        self.wrap_platform_object(self.node_platform_key(node), InterfaceBrand::NODE)
    }
    /// Live, same-object NodeList; indexed access, length and item read the arena.
    fn child_nodes(&mut self, parent: NodeIdx) -> Result<Self::Value, Self::Error> {
        self.wrap_platform_object(self.node_platform_key(parent), InterfaceBrand::NODE_LIST)
    }
    fn is_html_document(&self, document: NodeIdx) -> bool;

    fn undefined(&self) -> Self::Value;
    fn null(&self) -> Self::Value;
    fn boolean(&self, value: bool) -> Self::Value;
    fn number(&self, value: f64) -> Self::Value;
    /// A backend that cannot yet represent every UTF-16 sequence must fail
    /// explicitly; it must never substitute U+FFFD for a lone surrogate.
    fn string(&self, value: &[u16]) -> Result<Self::Value, Self::Error>;
    fn is_undefined(&self, value: &Self::Value) -> bool;
    fn is_nullish(&self, value: &Self::Value) -> bool;
    fn to_boolean(&self, value: &Self::Value) -> bool;
    /// Returns code units only when the runtime value is a primitive String.
    /// Used before WebIDL union conversion for `HTMLElement.hidden`.
    fn string_units(&self, value: &Self::Value) -> Option<Vec<u16>>;
    /// WebIDL DOMString conversion, preserving code units and thrown coercions.
    fn to_dom_string(&mut self, value: &Self::Value) -> Result<Vec<u16>, Self::Error>;
    fn type_error(&mut self, message: &'static str) -> Self::Error;
}
