//! Runtime-owned values and installation hooks; this crate never imports JS.
use amazing_dom::{Dom, DomError, NodeIdx};
pub use amazing_dom::events::CallbackId;
/// Opaque driver-owned strong reference to a JS value. Native state stores
/// only this token; the driver maps it to a value outside a DOM borrow.
pub type RootId = CallbackId;

/// Snapshot supplied by the embedding's image loader for HTMLImageElement.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImageLoaderState {
    pub current_src: Vec<u16>,
    pub complete: bool,
    pub natural_width: u32,
    pub natural_height: u32,
}

#[derive(Clone, Debug)]
pub struct FormNavigationRequest {
    pub url: String,
    pub method: String,
    pub enctype: String,
    pub target: String,
    pub body: Vec<u8>,
    pub content_type: Option<String>,
}

/// Result of an optional installation capability. `NotSupported` leaves the
/// rest of the interface usable and is recorded by the installer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionalHookInstall {
    Installed,
    NotSupported,
}

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
    pub const ABORT_CONTROLLER: Self = Self(19);
    pub const ABORT_SIGNAL: Self = Self(20);
    pub const FILTERED_NODE_LIST: Self = Self(21);
    pub const CUSTOM_ELEMENT_REGISTRY: Self = Self(22);
    pub const FORM_CONTROLS: Self = Self(23);
    pub const OPTIONS: Self = Self(24);
    pub const RADIO_NODE_LIST: Self = Self(25);
    pub const VALIDITY_STATE: Self = Self(26);
    pub const URL: Self = Self(27);
    pub const URL_SEARCH_PARAMS: Self = Self(28);
    pub const BLOB: Self = Self(29);
    pub const FILE: Self = Self(30);
    pub const FORM_DATA: Self = Self(31);
    pub const FONT_FACE_SET: Self = Self(32);
    pub const DOM_PARSER: Self = Self(33);
    pub const LOCATION: Self = Self(34);
    pub const DOM_STRING_LIST: Self = Self(35);
    pub const SELECTION: Self = Self(36);
    pub const SELECTED_OPTIONS: Self = Self(37);
    pub const DOM_STRING_MAP: Self = Self(38);
    pub const OBSERVABLE: Self = Self(39);
    pub const SUBSCRIBER: Self = Self(40);
    pub const ANIMATION: Self = Self(41);
    pub const CLIPBOARD: Self = Self(42);
    pub const CSS_STYLE_DECLARATION: Self = Self(43);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Interface {
    EventTarget,
    Node,
    CDATASection,
    Attr,
    NamedNodeMap,
    DOMTokenList,
    DOMStringMap,
    Location,
    DOMStringList,
    Document,
    XMLDocument,
    DocumentFragment,
    ShadowRoot,
    DocumentType,
    Element,
    HTMLElement,
    SVGElement,
    MathMLElement,
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
    HTMLSpanElement,
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
    HTMLDirectoryElement,
    HTMLFontElement,
    HTMLFrameElement,
    HTMLFrameSetElement,
    HTMLCollection,
    CharacterData,
    Text,
    Comment,
    ProcessingInstruction,
    AbstractRange,
    StaticRange,
    Range,
    Selection,
    DOMImplementation,
    NodeFilter,
    NodeList,
    NodeIterator,
    TreeWalker,
    Event,
    CustomEvent,
    UIEvent,
    FocusEvent,
    MouseEvent,
    WheelEvent,
    KeyboardEvent,
    CompositionEvent,
    HashChangeEvent,
    PopStateEvent,
    PageTransitionEvent,
    BeforeUnloadEvent,
    ErrorEvent,
    PromiseRejectionEvent,
    MessageEvent,
    DeviceMotionEvent,
    DeviceOrientationEvent,
    DragEvent,
    StorageEvent,
    TextEvent,
    MutationObserver,
    MutationRecord,
    DOMException,
    AbortController,
    AbortSignal,
    CustomElementRegistry,
    HTMLFormControlsCollection,
    HTMLOptionsCollection,
    RadioNodeList,
    ValidityState,
    URL,
    URLSearchParams,
    Blob,
    File,
    FormData,
    FontFaceSet,
    DOMParser,
    Observable,
    Subscriber,
    Animation,
    Clipboard,
    CSSStyleDeclaration,
    CSSStyleProperties,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormAttribute {
    Accept, Alpha, Alt, Autocomplete, DirName, Disabled, FormAction,
    FormEnctype, FormMethod, FormNoValidate, FormTarget, Height, Max,
    MaxLength, Min, MinLength, Multiple, Name, Pattern, Placeholder,
    ReadOnly, Required, Size, Src, Step, Width, Action, AcceptCharset,
    Enctype, Encoding, Method, NoValidate, Target, Rel, RelList,
    Cols, Rows, Wrap, Label, Value, Command, HtmlFor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LegacyReflectionKind {
    String, LegacyString, Url, UsvUrl, Boolean, Unsigned, Signed,
    /// A `DOMString` attribute "limited to only known values" whose missing
    /// value default is the empty string (unlike `Loading`/`Decoding` in
    /// `element_reflection.rs`'s own `Kind`, which default to a specific
    /// keyword instead -- those stay local since nothing else needs this
    /// exact "" fallback yet). Getter: the attribute value if it
    /// ASCII-case-insensitively matches one of `keywords`, else "". Setter:
    /// plain passthrough, same as `String` -- WebIDL's reflection algorithm
    /// only limits the *getter*, per
    /// <https://html.spec.whatwg.org/multipage/common-dom-interfaces.html#reflect>.
    /// First user: `HTMLTemplateElement.shadowRootMode` (DOM-18 follow-up).
    LimitedKnownValues(&'static [&'static str]),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Cssom(crate::cssom::CssomOp),
    /// `true` when the click is driver-initiated (e.g. testdriver's
    /// synthetic `mousedown`/`mouseup`/click sequence, simulating real user
    /// input) and should dispatch as a trusted event; `false` for an
    /// ordinary script call to `element.click()`, which the HTML Standard
    /// always dispatches untrusted.
    HTMLElementClick(bool),
    HTMLElementOnClick,
    HTMLElementSetOnClick,
    InputIndeterminate,
    InputSetIndeterminate,
    ErrorEventConstructor,
    PromiseRejectionEventConstructor,
    ErrorEventGet(&'static str),
    PromiseRejectionEventGet(&'static str),
    LegacyReflectGet(Interface, &'static str, LegacyReflectionKind),
    LegacyReflectSet(Interface, &'static str, LegacyReflectionKind),
    FormAttributeGet(FormAttribute), FormAttributeSet(FormAttribute),
    URLConstructor, URLCanParse, URLParse, URLHref, URLSetHref,
    URLOrigin, URLProtocol, URLSetProtocol, URLUsername, URLSetUsername,
    URLPassword, URLSetPassword, URLHost, URLSetHost, URLHostname,
    URLSetHostname, URLPort, URLSetPort, URLPathname, URLSetPathname,
    URLSearch, URLSetSearch, URLSearchParams, URLHash, URLSetHash,
    URLToString, URLToJSON,
    URLSearchParamsConstructor, URLSearchParamsAppend, URLSearchParamsDelete,
    URLSearchParamsGet, URLSearchParamsGetAll, URLSearchParamsHas,
    URLSearchParamsSet, URLSearchParamsSort, URLSearchParamsSize,
    URLSearchParamsToString, URLSearchParamsForEach,
    BlobConstructor, BlobSize, BlobType, BlobSlice, BlobText,
    BlobArrayBuffer, BlobBytes, BlobStream,
    FileConstructor, FileName, FileLastModified,
    FormDataConstructor, FormDataAppend, FormDataDelete, FormDataGet,
    FormDataGetAll, FormDataHas, FormDataSet, FormDataForEach,
    DocumentFonts, FontFaceSetStatus, FontFaceSetReady,
    DOMParserConstructor, DOMParserParseFromString,
    DocumentLocation, LocationHref, LocationSetHref, LocationOrigin, LocationProtocol, LocationSetProtocol,
    LocationHost, LocationSetHost, LocationHostname, LocationSetHostname,
    LocationPort, LocationSetPort, LocationPathname, LocationSetPathname,
    LocationSearch, LocationSetSearch, LocationHash, LocationSetHash,
    LocationAssign, LocationReplace, LocationReload, LocationAncestorOrigins,
    LocationToString, DOMStringListLength, DOMStringListItem, DOMStringListContains,
    FormElements, FormLength, FormReset, FormSubmit, FormRequestSubmit,
    FormCheckValidity, FormReportValidity,
    FormAutocompleteGet, FormAutocompleteSet, FormDataFormConstructor,
    FormControlForm, FormControlName, FormControlSetName,
    InputValue, InputSetValue, InputDefaultValue, InputSetDefaultValue,
    InputChecked, InputSetChecked, InputDefaultChecked, InputSetDefaultChecked,
    InputValueAsNumber, InputSetValueAsNumber, InputValueAsDate, InputSetValueAsDate,
    InputStepUp, InputStepDown,
    InputSelect, InputSetSelectionRange, InputSetRangeText, InputSelectionStart, InputSetSelectionStart,
    InputSelectionEnd, InputSetSelectionEnd, InputSelectionDirection, InputSetSelectionDirection,
    TextAreaValue, TextAreaSetValue, TextAreaDefaultValue, TextAreaSetDefaultValue,
    SelectValue, SelectSetValue, SelectSelectedIndex, SelectSetSelectedIndex,
    SelectOptions, SelectLength, SelectMultiple, SelectSetMultiple, SelectAdd, SelectRemove,
    OptionValue, OptionSetValue, OptionSelected, OptionSetSelected,
    SelectItem, SelectNamedItem, SelectSetLength, SelectSelectedOptions,
    SelectedOptionsIndexedGetter, SelectedOptionsSupportedIndexCount,
    SelectedOptionsNamedGetter,
    OptionDefaultSelected, OptionSetDefaultSelected, OptionText, OptionSetText,
    OptionIndex,
    ButtonType, ButtonSetType,
    ControlWillValidate, ControlValidity, ControlValidationMessage,
    ControlCheckValidity, ControlReportValidity, ControlSetCustomValidity,
    ValidityValueMissing, ValidityTypeMismatch, ValidityPatternMismatch,
    ValidityTooLong, ValidityTooShort, ValidityRangeUnderflow, ValidityRangeOverflow,
    ValidityStepMismatch, ValidityBadInput, ValidityCustomError, ValidityValid,
    FormCollectionLength, FormCollectionItem, FormCollectionNamedItem,
    FormCollectionIndexedGetter, FormCollectionNamedGetter,
    FormCollectionSupportedIndexCount, FormCollectionSupportedNames,
    OptionsLength, OptionsSetLength, OptionsSelectedIndex, OptionsSetSelectedIndex,
    OptionsItem, OptionsNamedItem, OptionsIndexedGetter, OptionsNamedGetter,
    OptionsIndexedSetter, OptionsSupportedIndexCount, OptionsSupportedNames,
    RadioNodeListLength, RadioNodeListItem, RadioNodeListIndexedGetter,
    RadioNodeListSupportedIndexCount, RadioNodeListValue, RadioNodeListSetValue,
    ElementGetHTML, ElementHref, ElementSetHref, ElementTarget, ElementSetTarget,
    HyperlinkOrigin, HyperlinkProtocol, HyperlinkSetProtocol,
    HyperlinkUsername, HyperlinkSetUsername, HyperlinkPassword, HyperlinkSetPassword,
    HyperlinkHost, HyperlinkSetHost, HyperlinkHostname, HyperlinkSetHostname,
    HyperlinkPort, HyperlinkSetPort, HyperlinkPathname, HyperlinkSetPathname,
    HyperlinkSearch, HyperlinkSetSearch, HyperlinkHash, HyperlinkSetHash,
    ElementRel, ElementSetRel, ElementRelList, ElementSetRelList,
    OutputHtmlFor, OutputSetHtmlFor, IFrameSandbox, IFrameSetSandbox,
    OutputValue, OutputSetValue, OutputDefaultValue, OutputSetDefaultValue,
    LabelControl, LabelForm,
    ProgressValue, ProgressSetValue, ProgressMax, ProgressSetMax, ProgressPosition,
    MeterValue, MeterSetValue, MeterMin, MeterSetMin, MeterMax, MeterSetMax,
    MeterLow, MeterSetLow, MeterHigh, MeterSetHigh, MeterOptimum, MeterSetOptimum,
    LinkSizes, LinkSetSizes, ElementDownload, ElementSetDownload,
    ElementSrc, ElementSetSrc, ImageAlt, ImageSetAlt, ImageWidth, ImageSetWidth,
    ImageHeight, ImageSetHeight, ImageLoading, ImageSetLoading,
    ImageSrcset, ImageSetSrcset, ImageSizes, ImageSetSizes, ImageCurrentSrc,
    ImageDecoding, ImageSetDecoding, ImageComplete, ImageNaturalWidth,
    ImageNaturalHeight, ImageDecode, SourceType, SourceSetType,
    SourceMedia, SourceSetMedia, SourceSrcset, SourceSetSrcset,
    SourceSizes, SourceSetSizes,
    InputType, InputSetType, ElementName, ElementSetName, ElementDisabled, ElementSetDisabled,
    InputPlaceholder, InputSetPlaceholder, ResourceType, ResourceSetType,
    ScriptAsync, ScriptSetAsync, ScriptDefer, ScriptSetDefer,
    LinkMedia, LinkSetMedia, MetaContent, MetaSetContent,
    MetaHttpEquiv, MetaSetHttpEquiv, MetaCharset, MetaSetCharset,
    TableCaption, TableSetCaption, TableCreateCaption, TableDeleteCaption,
    TableHead, TableSetHead, TableCreateHead, TableDeleteHead,
    TableFoot, TableSetFoot, TableCreateFoot, TableDeleteFoot,
    TableBodies, TableCreateBody, TableRows, TableInsertRow, TableDeleteRow,
    SectionRows, SectionInsertRow, SectionDeleteRow,
    RowIndex, RowSectionIndex, RowCells, RowInsertCell, RowDeleteCell, CellIndex,
    CellColSpan, CellSetColSpan, CellRowSpan, CellSetRowSpan,
    OListStart, OListSetStart, OListReversed, OListSetReversed,
    LIValue, LISetValue,
    DirectoryCompact,
    DirectoryCompactSet,
    FontColor,
    FontColorSet,
    FontFace,
    FontFaceSet,
    FontSize,
    FontSizeSet,
    FrameSetCols,
    FrameSetColsSet,
    FrameSetRows,
    FrameSetRowsSet,
    FrameName,
    FrameNameSet,
    FrameScrolling,
    FrameScrollingSet,
    FrameSrc,
    FrameSrcSet,
    FrameBorder,
    FrameBorderSet,
    FrameLongDesc,
    FrameLongDescSet,
    FrameNoResize,
    FrameNoResizeSet,
    FrameMarginHeight,
    FrameMarginHeightSet,
    FrameMarginWidth,
    FrameMarginWidthSet,
    DocumentLegacyColorGet(amazing_dom::legacy_document::LegacyColor),
    DocumentLegacyColorSet(amazing_dom::legacy_document::LegacyColor),
    BodyLegacyColorGet(amazing_dom::legacy_document::LegacyColor),
    BodyLegacyColorSet(amazing_dom::legacy_document::LegacyColor),
    DocumentLegacyNoop, WindowLegacyNoop,
    DocumentTitle,
    DocumentSetTitle,
    DocumentHead,
    DocumentBody,
    DocumentSetBody,
    DocumentElement,
    DocumentDoctype,
    DocumentReadyState,
    DocumentDefaultView,
    DocumentURL,
    DocumentURI,
    DocumentCharacterSet,
    DocumentContentType,
    DocumentCompatMode,
    DocumentReferrer,
    DocumentGetElementById,
    FragmentGetElementById,
    DocumentGetElementsByName,
    DocumentAdoptNode,
    DocumentImportNode,
    DocumentForms,
    DocumentImages,
    DocumentEmbeds, DocumentAnchors, DocumentApplets,
    DocumentLinks,
    DocumentScripts,
    DocumentActiveElement,
    DocumentHasFocus,
    ElementFocus,
    ElementBlur,
    AbortControllerConstructor,
    AbortControllerSignal,
    AbortControllerAbort,
    AbortSignalAborted,
    AbortSignalReason,
    AbortSignalThrowIfAborted,
    AbortSignalOnAbort,
    AbortSignalSetOnAbort,
    AbortSignalAbort,
    AbortSignalAny,
    AbortSignalTimeout,
    ElementInnerHTML,
    TemplateContent,
    ElementSetInnerHTML,
    /// `Element`/`ShadowRoot.setHTMLUnsafe(html, options?)` (HTML sanitizer
    /// API). Routed to the exact same fragment-replacement body as
    /// `ElementSetInnerHTML` in `markup_bindings.rs`: this engine has no
    /// sanitizer to apply either way (so "unsafe" -- skip sanitizing -- and
    /// the ordinary setter currently behave identically), and neither path
    /// yet parses with declarative shadow roots enabled --
    /// `amazing_html::parse_fragment` hardcodes `allow_declarative_shadow_
    /// roots = false` for every fragment context, a parser-level flag only
    /// html lane's crate can add (routed to `questions.md`). Exists mainly
    /// so the method is callable at all -- previously entirely absent, so
    /// any WPT test calling it (e.g. `shadow-dom/declarative/declarative-
    /// shadow-dom-basic.html`) crashed with `TypeError: value is not
    /// callable` before running any of its assertions.
    ElementSetHTMLUnsafe,
    /// Static `Document.parseHTMLUnsafe(html, options?)`. See
    /// `parser_bindings.rs`'s handling for why this needs no new parser
    /// support beyond `create_inert_document`, unlike `ElementSetHTMLUnsafe`.
    DocumentParseHTMLUnsafe,
    ElementOuterHTML,
    ElementSetOuterHTML,
    ElementInsertAdjacentHTML,
    ElementInsertAdjacentElement,
    ElementInsertAdjacentText,
    ElementInnerText,
    ElementSetInnerText,
    GetRootNode,
    NodeBaseURI,
    MoveBefore,
    CustomElementRegistryDefine,
    CustomElementRegistryGet,
    CustomElementRegistryGetName,
    CustomElementRegistryWhenDefined,
    CustomElementRegistryUpgrade,
    ElementAttachShadow,
    ElementShadowRoot,
    ElementAssignedSlot,
    ElementSlot,
    ElementSetSlot,
    TextAssignedSlot,
    ShadowHost,
    ShadowMode,
    ShadowDelegatesFocus,
    ShadowSlotAssignment,
    ShadowClonable,
    ShadowSerializable,
    SlotName,
    SlotSetName,
    SlotAssign,
    SlotAssignedNodes,
    SlotAssignedElements,
    ParentQuerySelector,
    ParentQuerySelectorAll,
    ElementMatches,
    ElementClosest,
    ElementWebkitMatchesSelector,
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
    ElementHasAttributes,
    ElementGetAttributeNames,
    ElementHasAttribute,
    ElementHasAttributeNS,
    ElementToggleAttribute,
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
    NamedNodeMapSupportedIndexCount,
    NamedNodeMapSupportedNames,
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
    TokenListSupportedIndexCount,
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
    NodeLookupNamespaceURI,
    NodeIsDefaultNamespace,
    NodeNormalize,
    NodeIsSameNode,
    NodeIsEqualNode,
    NodeCompareDocumentPosition,
    ElementNamespaceURI,
    ElementPrefix,
    ElementLocalName,
    ElementTagName,
    DocumentTypeName,
    DocumentTypePublicId,
    DocumentTypeSystemId,
    ProcessingInstructionTarget,
    ProcessingInstructionConstructor,
    PiHasAttributes,
    PiGetAttributeNames,
    PiGetAttribute,
    PiSetAttribute,
    PiRemoveAttribute,
    PiToggleAttribute,
    PiHasAttribute,

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
    CharacterDataSubstring,
    CharacterDataAppend,
    CharacterDataInsert,
    CharacterDataDelete,
    CharacterDataReplace,
    TextSplit,
    TextWholeText,
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
    ElementDataset, DatasetGet, DatasetSet, DatasetDelete, DatasetIndexedGet,
    DatasetIndexCount, DatasetSupportedNames,
    GetSelection, SelectionAnchorNode, SelectionAnchorOffset,
    SelectionFocusNode, SelectionFocusOffset, SelectionIsCollapsed,
    SelectionRangeCount, SelectionGetRangeAt, SelectionAddRange,
    SelectionRemoveRange, SelectionRemoveAllRanges, SelectionCollapse,
    SelectionSelectAllChildren, SelectionToString,
    DocumentImplementation,
    ImplementationCreateDocumentType,
    ImplementationCreateDocument,
    ImplementationCreateHTMLDocument,
    ImplementationHasFeature,
    CreateElement,
    CreateElementNS,
    NodeListLength,
    NodeListItem,
    NodeListSupportedIndexCount,
    NodeListIndexedGetter,
    ElementChildren,
    DocumentChildren,
    FragmentChildren,
    FirstElementChild,
    LastElementChild,
    ChildElementCount,
    PreviousElementSibling,
    NextElementSibling,
    ElementGetElementsByTagName,
    ElementGetElementsByTagNameNS,
    ElementGetElementsByClassName,
    DocumentGetElementsByTagName,
    DocumentGetElementsByTagNameNS,
    DocumentGetElementsByClassName,
    HTMLCollectionLength,
    HTMLCollectionItem,
    HTMLCollectionNamedItem,
    HTMLCollectionSupportedIndexCount,
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
    UIEventConstructor,
    FocusEventConstructor,
    MouseEventConstructor,
    WheelEventConstructor,
    KeyboardEventConstructor,
    CompositionEventConstructor,
    HashChangeEventConstructor,
    PopStateEventConstructor,
    PageTransitionEventConstructor,
    HashChangeOldURL,
    HashChangeNewURL,
    PopStateState,
    PopStateHasUAVisualTransition,
    PageTransitionPersisted,
    BeforeUnloadReturnValue,
    BeforeUnloadSetReturnValue,

    MessageEventConstructor,
    MessageEventGet(&'static str),
    MessageEventInit,
    EventExtra(&'static str),
    MouseGetModifierState,
    KeyboardGetModifierState,
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

    ObservableConstructor,
    ObservableSubscribe,
    ObservableMap,
    ObservableFilter,
    ObservableTake,
    ObservableDrop,
    ObservableFlatMap,
    ObservableSwitchMap,
    ObservableTakeUntil,
    ObservableForEach,
    ObservableToArray,
    ObservableFirst,
    ObservableLast,
    ObservableFind,
    ObservableSome,
    ObservableEvery,
    ObservableReduce,
    ObservableFrom,
    SubscriberNext,
    SubscriberError,
    SubscriberComplete,
    SubscriberAddTeardown,
    SubscriberActive,
    SubscriberSignal,
    EventTargetWhen,
    /// Internal glue functions created dynamically with `create_native_function`,
    /// never installed on a prototype. The `u64` packs (ObservableId, run
    /// generation, and for the promise pair which settlement branch) so one
    /// thunk shape covers relay/teardown/event-listener/promise-reaction use.
    ObservableInternalThunk(u64),
    /// ARIAMixin reflection (DOM-B-REFLECT): `role` plus the whole aria-*
    /// string-attribute set, all `DOMString?` reflections of the named
    /// content attribute, applying to every Element regardless of namespace
    /// or tag -- unlike `LegacyReflectGet`/`Set`, which gate on a specific
    /// tag/interface. See `aria_reflection.rs`.
    AriaGet(&'static str),
    AriaSet(&'static str),
    /// `Element.prototype.animate()` (Animatable mixin) and the minimal
    /// `Animation` object it returns. See `animation_bindings.rs`.
    ElementAnimate,
    AnimationFinished,
    AnimationReady,
    AnimationCancel,
    AnimationFinish,
    AnimationPlay,
    AnimationPause,
    AnimationOnFinish,
    AnimationSetOnFinish,
    /// `navigator.clipboard` and `Clipboard.readText`/`writeText`, backed by
    /// an in-memory clipboard (`Dom::clipboard_text`). See `clipboard_bindings.rs`.
    NavigatorClipboard,
    ClipboardReadText,
    ClipboardWriteText,
}

/// Context passed to the embedding's HTML fragment parser after the DOM borrow
/// has ended. Parser results live in a separate arena and are imported by DOM.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FragmentContext {
    /// Exact owner document scripting state, captured before parsing.
    pub scripting_enabled: bool,
    pub local_name: String,
    pub namespace_uri: Option<String>,
}

pub trait FragmentParser: BindingHost {
    fn parse_fragment(&mut self, context: FragmentContext, markup: &[u16])
        -> Result<Dom, DomError>;
    /// `Element`/`ShadowRoot.setHTMLUnsafe` and `Document.parseHTML(Unsafe)`
    /// need declarative shadow roots enabled during fragment parsing (spec's
    /// "set and filter HTML" always parses with `allowDeclarativeShadowRoots
    /// = true`); ordinary `innerHTML`/`outerHTML`/`insertAdjacentHTML` must
    /// keep them off and keep calling plain `parse_fragment`. Defaults to
    /// `parse_fragment` (declarative shadow roots off) so a host that
    /// hasn't opted in keeps its current, pre-existing behavior.
    fn parse_fragment_with_shadow_roots(&mut self, context: FragmentContext, markup: &[u16])
        -> Result<Dom, DomError> {
        self.parse_fragment(context, markup)
    }
}

/// The embedding parses and matches selectors using its CSS crate. A failed
/// parse must surface as SyntaxError; handles are private to that embedding.
pub trait SelectorEngine: BindingHost {
    type SelectorHandle;
    fn parse_selector(&mut self, selector: &[u16]) -> Result<Self::SelectorHandle, DomError>;
    fn matches_selector(&mut self, element: NodeIdx, selector: &Self::SelectorHandle)
        -> Result<bool, DomError>;
    /// Like `matches_selector`, but binds `:scope` to `scope` instead of the
    /// document root. `Element.closest()`'s DOM algorithm matches each
    /// inclusive ancestor against the selector list with `:scope` bound to
    /// the ORIGINAL context element throughout the walk, not to whichever
    /// ancestor is currently being tested (CSS-29 made the primitive this
    /// needs, `amazing_css::selectors::matches_in_scope`, crate-external
    /// `pub`). Hosts that don't override this fall back to plain
    /// `matches_selector` (a `:scope` compound then matches nothing but the
    /// document root, the pre-existing behavior).
    fn matches_selector_scoped(&mut self, element: NodeIdx, selector: &Self::SelectorHandle, _scope: NodeIdx)
        -> Result<bool, DomError> {
        self.matches_selector(element, selector)
    }
}

/// Driver schedules timeouts and runs native fetch/XHR abort subscribers after
/// the DOM borrow is released. It calls `fire_abort_timeout` at timer expiry.
pub trait AbortHost: BindingHost {
    fn schedule_abort_timeout(&mut self, signal: amazing_dom::abort::AbortSignalId, milliseconds: u64)
        -> Result<(), Self::Error>;
    fn run_abort_algorithm(&mut self, algorithm: u64, reason: &Self::Value)
        -> Result<(), Self::Error>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocumentField { URL, DocumentURI, CharacterSet, ContentType, CompatMode, Referrer }

/// Browsing-context data comes from the driver, not guesses from the DOM tree.
pub trait DocumentHost: BindingHost {
    /// Effective base of this actual document, including embedding fallback.
    fn document_base_url(&mut self, document: NodeIdx) -> Result<Vec<u16>, Self::Error>
    where Self: UrlReflection {
        let href = self.with_dom(|dom| dom.document_base_href(document))?;
        match href {
            Some(href) => self.resolve_url(&href),
            None => self.document_field(document, DocumentField::URL),
        }
    }
    /// The document's associated WindowProxy, or None for an inert document.
    /// Child-realm embedders supply the proxy belonging to that document.
    fn document_default_view(&mut self, _document: NodeIdx) -> Result<Option<Self::Value>, Self::Error> {
        self.with_dom::<Option<Self::Value>>(|_| Err(DomError::NotSupportedError))
    }
    fn document_field(&mut self, document: NodeIdx, field: DocumentField)
        -> Result<Vec<u16>, Self::Error>;
    fn active_element(&mut self, document: NodeIdx) -> Result<Option<NodeIdx>, Self::Error>;
    fn has_focus(&mut self, document: NodeIdx) -> Result<bool, Self::Error>;
}

/// URL-valued reflected attributes resolve against the embedding's document
/// base URL without making bindings depend on the URL/network crate.
pub trait UrlReflection: BindingHost {
    fn resolve_url(&mut self, input: &[u16]) -> Result<Vec<u16>, Self::Error>;
    fn resolve_element_url(&mut self, _element: NodeIdx, input: &[u16]) -> Result<Vec<u16>, Self::Error> {
        self.resolve_url(input)
    }
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
    /// Routed through `supported_index_count`, returning a native count.
    pub supported_index_count: Operation,
    /// Routed through `supported_names`, returning UTF-16 names in spec order.
    pub supported_names: Option<Operation>,
    pub indexed_getter: Operation,
    pub indexed_setter: Option<Operation>,
    pub named_getter: Option<Operation>,
    pub named_setter: Option<Operation>,
    pub named_deleter: Option<Operation>,
    pub override_builtins: bool,
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

    /// Optional CSSOM dispatch seam; a host installs its real style callback.
    fn invoke_cssom(&mut self, _operation: crate::cssom::CssomOp, _receiver: Self::Value,
        _arguments: &[Self::Value]) -> Result<Self::Value, Self::Error> {
        Err(self.type_error("CSSOM host callback is not installed"))
    }
    fn install_cssom(&mut self) -> Result<OptionalHookInstall, Self::Error> {
        Ok(OptionalHookInstall::NotSupported)
    }

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
    /// Pair iterables need a runtime iterator whose values are live [key, value]
    /// arrays. Older runtimes report this optional capability as unsupported.
    fn define_pair_iterable(&mut self, _interface: Interface) -> Result<OptionalHookInstall, Self::Error> {
        Ok(OptionalHookInstall::NotSupported)
    }
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
    /// The embedding's Window is an EventTarget with a stable external slot.
    fn window_event_target_slot(&self, _value: &Self::Value) -> Option<u64> { None }
    fn window_event_target_value(&self, _slot: u64) -> Option<Self::Value> { None }
    /// Embedding-owned targets (e.g. MessagePort) use Dom::create_event_target slots.
    /// Validate private wrapper identity, realm and liveness before returning a slot.
    fn external_event_target_slot(&self, _value: &Self::Value) -> Option<u64> { None }
    fn external_event_target_value(&self, _slot: u64) -> Option<Self::Value> { None }
    fn is_message_port(&self, _value: &Self::Value) -> bool { false }
    fn is_message_event_source(&self, value: &Self::Value) -> bool {
        !self.is_nullish(value) && (self.window_event_target_slot(value).is_some()
            || self.is_message_port(value))
    }
    /// WebIDL iterable conversion, including MessagePort brand checks and IteratorClose.
    fn sequence_message_ports(&mut self, _value: &Self::Value) -> Result<Vec<Self::Value>, Self::Error> {
        self.with_dom(|_| Err(DomError::NotSupportedError))
    }
    /// FrozenArray conversion must not call a script-replaceable Object.freeze.
    fn create_frozen_array(&mut self, _values: Vec<Self::Value>) -> Result<Self::Value, Self::Error> {
        self.with_dom(|_| Err(DomError::NotSupportedError))
    }
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
    /// Define the prototype's real %Symbol.toStringTag% data property using
    /// the interface's qualified name. A string-keyed lookalike is invalid.
    /// An older driver returns typed `NotSupported` until it gains symbol keys.
    fn install_interface_class_string(&mut self, _interface: Interface, _name: &'static str)
        -> Result<OptionalHookInstall, Self::Error> {
        Ok(OptionalHookInstall::NotSupported)
    }
    /// DOMException.prototype inherits %Error.prototype%, including its
    /// name/message toString behavior. Kept separate from WebIDL inheritance.
    fn install_dom_exception_error_prototype(&mut self) -> Result<OptionalHookInstall, Self::Error> {
        Ok(OptionalHookInstall::NotSupported)
    }
    /// Expose the current document's registry as `window.customElements`.
    fn install_custom_elements_global(&mut self) -> Result<OptionalHookInstall, Self::Error> {
        Ok(OptionalHookInstall::NotSupported)
    }
    fn is_constructor(&mut self, _value: &Self::Value) -> Result<bool, Self::Error> {
        self.with_dom::<bool>(|_| Err(DomError::NotSupportedError))
    }
    /// Classify the WebIDL object branch without invoking user code.
    fn is_object_value(&mut self, _value: &Self::Value) -> Result<bool, Self::Error> {
        self.with_dom::<bool>(|_| Err(DomError::NotSupportedError))
    }
    fn require_object(&mut self, _value: &Self::Value) -> Result<(), Self::Error> {
        self.with_dom::<()>(|_| Err(DomError::NotSupportedError))
    }
    fn sequence_dom_strings(&mut self, _value: &Self::Value) -> Result<Vec<Vec<u16>>, Self::Error> {
        self.with_dom::<Vec<Vec<u16>>>(|_| Err(DomError::NotSupportedError))
    }
    /// Array-like-to-`Vec<Value>` conversion (`.length` plus indexed reads),
    /// used by `Observable.from()` for its Array/array-like input case. This
    /// is not the full iterator protocol: a genuine custom iterable or
    /// generator is not walked, only its `.length`/indexed shape if it has one.
    fn sequence_values(&mut self, _value: &Self::Value) -> Result<Vec<Self::Value>, Self::Error> {
        self.with_dom::<Vec<Self::Value>>(|_| Err(DomError::NotSupportedError))
    }
    /// WebIDL sequence<ShadowRoot> for getHTML({shadowRoots}).
    fn sequence_shadow_roots(&mut self, _value: &Self::Value) -> Result<Vec<NodeIdx>, Self::Error> {
        self.with_dom::<Vec<NodeIdx>>(|_| Err(DomError::NotSupportedError))
    }
    fn construct_custom_element(&mut self, _constructor: &Self::Value,
        _definition: usize, _element: NodeIdx) -> Result<Self::Value, Self::Error> {
        self.with_dom::<Self::Value>(|_| Err(DomError::NotSupportedError))
    }
    fn new_promise(&mut self) -> Result<Self::Value, Self::Error> {
        self.with_dom::<Self::Value>(|_| Err(DomError::NotSupportedError))
    }
    fn resolve_promise(&mut self, _promise: &Self::Value, _value: Self::Value) -> Result<(), Self::Error> {
        self.with_dom::<()>(|_| Err(DomError::NotSupportedError))
    }
    fn reject_promise(&mut self, _promise: &Self::Value, _reason: Self::Value) -> Result<(), Self::Error> {
        self.with_dom::<()>(|_| Err(DomError::NotSupportedError))
    }
    fn dom_exception_value(&mut self, _error: DomError) -> Result<Self::Value, Self::Error> {
        self.with_dom::<Self::Value>(|_| Err(DomError::NotSupportedError))
    }
    /// A constructed (not thrown) `RangeError` value, for spots like
    /// `Observable#first()`/`last()`/`reduce()` that reject a promise with a
    /// real `RangeError` rather than a `DOMException`.
    fn range_error_value(&mut self, _message: &'static str) -> Result<Self::Value, Self::Error> {
        self.with_dom::<Self::Value>(|_| Err(DomError::NotSupportedError))
    }
    fn report_custom_element_exception(&mut self, _error: Self::Error) -> Result<(), Self::Error> {
        self.with_dom::<()>(|_| Err(DomError::NotSupportedError))
    }
    /// Full-string `pattern` match using the runtime's JavaScript `v` regexp.
    fn form_pattern_matches(&mut self, _pattern: &[u16], _value: &[u16]) -> Result<Option<bool>, Self::Error> {
        self.with_dom::<Option<bool>>(|_| Err(DomError::NotSupportedError))
    }
    /// Runtime Date wrapper, preserving the Date value in milliseconds.
    fn form_date_from_epoch_ms(&mut self, _milliseconds: f64) -> Result<Self::Value, Self::Error> {
        self.with_dom::<Self::Value>(|_| Err(DomError::NotSupportedError))
    }
    /// A Date value returns its time value; invalid Date returns None.
    fn form_date_to_epoch_ms(&mut self, _value: &Self::Value) -> Result<Option<f64>, Self::Error> {
        self.with_dom::<Option<f64>>(|_| Err(DomError::NotSupportedError))
    }
    /// Driver-owned storage for URL and body objects. Defaults are explicitly
    /// unsupported until the runtime adapter can allocate their private slots.
    /// Current browsing-context URL; an absent browsing context uses about:blank.
    fn location_href(&mut self) -> Result<String, Self::Error> {
        self.with_dom::<String>(|_| Err(DomError::NotSupportedError))
    }
    fn document_location(&mut self, _document: NodeIdx) -> Result<Option<Self::Value>, Self::Error> {
        self.with_dom::<Option<Self::Value>>(|_| Err(DomError::NotSupportedError))
    }
    /// Queue navigation after WebIDL coercion and URL parsing. `replace` selects
    /// history replacement rather than a new history entry.
    fn location_navigate(&mut self, _url: &str, _replace: bool) -> Result<(), Self::Error> {
        self.with_dom::<()>(|_| Err(DomError::NotSupportedError))
    }
    fn location_reload(&mut self) -> Result<(), Self::Error> {
        self.with_dom::<()>(|_| Err(DomError::NotSupportedError))
    }
    /// Queue a user-interaction task after a text control's selection changes.
    fn form_selection_changed(&mut self, _node: NodeIdx) -> Result<(), Self::Error> { Ok(()) }
    fn form_submit_event(&mut self, _form: NodeIdx, _submitter: Option<NodeIdx>)
        -> Result<bool, Self::Error> { Ok(true) }
    fn form_data_event(&mut self, _form: NodeIdx, data: amazing_fetch::api::FormData)
        -> Result<amazing_fetch::api::FormData, Self::Error> { Ok(data) }
    fn form_navigate(&mut self, request: FormNavigationRequest) -> Result<(), Self::Error> {
        self.location_navigate(&request.url, false)
    }
    fn location_ancestor_origins(&mut self) -> Result<Self::Value, Self::Error> {
        self.with_dom::<Self::Value>(|_| Err(DomError::NotSupportedError))
    }
    fn url_new(&mut self, _url: amazing_net::url::UrlWithSearchParams) -> Result<Self::Value, Self::Error> {
        self.with_dom::<Self::Value>(|_| Err(DomError::NotSupportedError))
    }
    fn url_record(&mut self, _receiver: &Self::Value) -> Result<amazing_net::url::UrlWithSearchParams, Self::Error> {
        self.with_dom::<amazing_net::url::UrlWithSearchParams>(|_| Err(DomError::NotSupportedError))
    }
    fn url_search_params(&mut self, _receiver: &Self::Value) -> Result<Self::Value, Self::Error> {
        self.with_dom::<Self::Value>(|_| Err(DomError::NotSupportedError))
    }
    fn url_params_new(&mut self, _params: amazing_net::url::UrlSearchParams) -> Result<Self::Value, Self::Error> {
        self.with_dom::<Self::Value>(|_| Err(DomError::NotSupportedError))
    }
    fn url_params_record(&mut self, _receiver: &Self::Value) -> Result<amazing_net::url::UrlSearchParams, Self::Error> {
        self.with_dom::<amazing_net::url::UrlSearchParams>(|_| Err(DomError::NotSupportedError))
    }
    fn url_params_store(&mut self, _receiver: &Self::Value, _params: amazing_net::url::UrlSearchParams) -> Result<(), Self::Error> {
        self.with_dom::<()>(|_| Err(DomError::NotSupportedError))
    }
    /// WebIDL sequence<sequence<USVString>> or record<USVString, USVString>.
    /// The adapter performs iterator/property access outside a DOM borrow.
    fn url_params_pairs(&mut self, _value: &Self::Value) -> Result<Option<Vec<(String, String)>>, Self::Error> {
        self.with_dom::<Option<Vec<(String, String)>>>(|_| Err(DomError::NotSupportedError))
    }
    fn blob_parts(&mut self, _value: &Self::Value) -> Result<Vec<amazing_fetch::api::BlobPart>, Self::Error> {
        self.with_dom::<Vec<amazing_fetch::api::BlobPart>>(|_| Err(DomError::NotSupportedError))
    }
    fn blob_new(&mut self, _blob: amazing_fetch::api::Blob) -> Result<Self::Value, Self::Error> {
        self.with_dom::<Self::Value>(|_| Err(DomError::NotSupportedError))
    }
    fn blob_record(&mut self, _receiver: &Self::Value) -> Result<amazing_fetch::api::Blob, Self::Error> {
        self.with_dom::<amazing_fetch::api::Blob>(|_| Err(DomError::NotSupportedError))
    }
    fn file_new(&mut self, _file: amazing_fetch::api::File) -> Result<Self::Value, Self::Error> {
        self.with_dom::<Self::Value>(|_| Err(DomError::NotSupportedError))
    }
    fn file_record(&mut self, _receiver: &Self::Value) -> Result<amazing_fetch::api::File, Self::Error> {
        self.with_dom::<amazing_fetch::api::File>(|_| Err(DomError::NotSupportedError))
    }
    fn form_data_new(&mut self, _data: amazing_fetch::api::FormData) -> Result<Self::Value, Self::Error> {
        self.with_dom::<Self::Value>(|_| Err(DomError::NotSupportedError))
    }
    fn form_data_record(&mut self, _receiver: &Self::Value) -> Result<amazing_fetch::api::FormData, Self::Error> {
        self.with_dom::<amazing_fetch::api::FormData>(|_| Err(DomError::NotSupportedError))
    }
    fn form_data_store(&mut self, _receiver: &Self::Value, _data: amazing_fetch::api::FormData) -> Result<(), Self::Error> {
        self.with_dom::<()>(|_| Err(DomError::NotSupportedError))
    }
    fn blob_promise_bytes(&mut self, _bytes: Vec<u8>, _as_text: bool, _as_array_buffer: bool) -> Result<Self::Value, Self::Error> {
        self.with_dom::<Self::Value>(|_| Err(DomError::NotSupportedError))
    }
    fn blob_stream(&mut self, _bytes: Vec<u8>) -> Result<Self::Value, Self::Error> {
        self.with_dom::<Self::Value>(|_| Err(DomError::NotSupportedError))
    }
    fn image_loader_state(&mut self, _node: NodeIdx) -> Result<Option<ImageLoaderState>, Self::Error> {
        Ok(None)
    }
    /// The loader settles `promise` when decoding completes. An embedding
    /// without image loading returns NotSupported for a rejected promise.
    fn image_decode(&mut self, _node: NodeIdx, _promise: &Self::Value) -> Result<OptionalHookInstall, Self::Error> {
        Ok(OptionalHookInstall::NotSupported)
    }
    fn font_face_set(&mut self, _document: NodeIdx) -> Result<Self::Value, Self::Error> {
        self.with_dom::<Self::Value>(|_| Err(DomError::NotSupportedError))
    }
    /// Absence of the provider hook means no pending loads.
    fn font_face_set_pending(&mut self, _receiver: &Self::Value) -> Result<bool, Self::Error> {
        Ok(false)
    }
    fn font_face_set_ready(&mut self, _receiver: &Self::Value) -> Result<Self::Value, Self::Error> {
        self.with_dom::<Self::Value>(|_| Err(DomError::NotSupportedError))
    }
    /// Standard Node constants are exposed on Node and its prototype.
    fn install_node_constant(&mut self, name: &'static str, value: u16) -> Result<(), Self::Error>;
    /// Constant on both an interface object and its prototype.
    fn install_interface_constant(
        &mut self,
        interface: Interface,
        name: &'static str,
        value: u32,
    ) -> Result<(), Self::Error>;
    fn install_static_method(&mut self, interface: Interface, name: &'static str,
        operation: Operation, length: u8) -> Result<(), Self::Error>;
    fn dom_parser_new(&mut self) -> Result<Self::Value, Self::Error> {
        self.with_dom(|_| Err(DomError::NotSupportedError))
    }
    /// Parses without script execution and imports into the active shared arena.
    /// SyntaxError means XML parsing failed; XHR returns null, while DOMParser
    /// turns that failure into an XML error document.
    fn create_inert_document(&mut self, _bytes: &[u8], _content_type: &str,
        _url: &str, _charset: Option<&str>) -> Result<NodeIdx, DomError> {
        Err(DomError::NotSupportedError)
    }
    fn current_document(&self) -> NodeIdx;
    /// Record the interface identity of `new Document()` before its wrapper is made.
    fn mark_constructed_document(&mut self, _document: NodeIdx) {}
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
