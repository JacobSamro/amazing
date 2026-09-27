//! Element-specific reflected attributes, using the shared DOM-8 converters.
use crate::{host::{Interface, InterfaceBrand, LegacyReflectionKind as L, Operation as O, UrlReflection}, html_reflection};
use amazing_dom::{DomError, Namespace, NodeKind};

#[derive(Clone, Copy)]
enum Kind { String, LegacyString, UsvUrl, Url, Boolean, Unsigned, ReflectedUnsigned, Signed(i32), InputType, Loading, Decoding, ColSpan, RowSpan, TokenList, LimitedKnownValues(&'static [&'static str]) }
#[derive(Clone, Copy)]
struct Spec { interface: Option<Interface>, attribute: &'static str, tags: &'static [&'static str], kind: Kind, setter: bool }
const HYPERLINK: &[&str] = &["a", "area"];
const REL: &[&str] = &["a", "area", "link"];
fn spec(op: O) -> Option<Spec> {
    use Kind as K;
    if let O::LegacyReflectGet(interface, attribute, kind) | O::LegacyReflectSet(interface, attribute, kind) = op {
        let kind = match kind { L::String => K::String, L::LegacyString => K::LegacyString,
            L::Url => K::Url, L::UsvUrl => K::UsvUrl, L::Boolean => K::Boolean,
            L::Unsigned => K::ReflectedUnsigned, L::Signed => K::Signed(0),
            L::LimitedKnownValues(keywords) => K::LimitedKnownValues(keywords) };
        return Some(Spec { interface: Some(interface), attribute, tags: &[], kind,
            setter: matches!(op, O::LegacyReflectSet(..)) });
    }
    let (attribute, tags, kind): (&'static str, &'static [&'static str], Kind) = match op {
        O::ElementHref | O::ElementSetHref => ("href", REL, K::Url),
        O::ElementTarget | O::ElementSetTarget => ("target", HYPERLINK, K::String),
        O::ElementRel | O::ElementSetRel => ("rel", REL, K::String),
        O::ElementRelList | O::ElementSetRelList => ("rel", REL, K::TokenList),
        O::OutputHtmlFor | O::OutputSetHtmlFor => ("for", &["output"], K::TokenList),
        O::IFrameSandbox | O::IFrameSetSandbox => ("sandbox", &["iframe"], K::TokenList),
        O::LinkSizes | O::LinkSetSizes => ("sizes", &["link"], K::TokenList),
        O::ElementDownload | O::ElementSetDownload => ("download", HYPERLINK, K::String),
        O::ElementSrc | O::ElementSetSrc => ("src", &["img", "script"], K::Url),
        O::ImageAlt | O::ImageSetAlt => ("alt", &["img"], K::String),
        O::ImageWidth | O::ImageSetWidth => ("width", &["img"], K::Unsigned),
        O::ImageHeight | O::ImageSetHeight => ("height", &["img"], K::Unsigned),
        O::ImageLoading | O::ImageSetLoading => ("loading", &["img"], K::Loading),
        O::ImageSrcset | O::ImageSetSrcset => ("srcset", &["img"], K::String),
        O::ImageSizes | O::ImageSetSizes => ("sizes", &["img"], K::String),
        O::ImageDecoding | O::ImageSetDecoding => ("decoding", &["img"], K::Decoding),
        O::SourceType | O::SourceSetType => ("type", &["source"], K::String),
        O::SourceMedia | O::SourceSetMedia => ("media", &["source"], K::String),
        O::SourceSrcset | O::SourceSetSrcset => ("srcset", &["source"], K::String),
        O::SourceSizes | O::SourceSetSizes => ("sizes", &["source"], K::String),
        O::InputType | O::InputSetType => ("type", &["input"], K::InputType),
        O::ElementName | O::ElementSetName => ("name", &["input", "meta"], K::String),
        O::ElementDisabled | O::ElementSetDisabled => ("disabled", &["input", "link"], K::Boolean),
        O::InputPlaceholder | O::InputSetPlaceholder => ("placeholder", &["input"], K::String),
        O::ResourceType | O::ResourceSetType => ("type", &["script", "link"], K::String),
        O::ScriptAsync | O::ScriptSetAsync => ("async", &["script"], K::Boolean),
        O::ScriptDefer | O::ScriptSetDefer => ("defer", &["script"], K::Boolean),
        O::LinkMedia | O::LinkSetMedia => ("media", &["link"], K::String),
        O::MetaContent | O::MetaSetContent => ("content", &["meta"], K::String),
        O::MetaHttpEquiv | O::MetaSetHttpEquiv => ("http-equiv", &["meta"], K::String),
        O::MetaCharset | O::MetaSetCharset => ("charset", &["meta"], K::String),
        O::CellColSpan | O::CellSetColSpan => ("colspan", &["td", "th"], K::ColSpan),
        O::CellRowSpan | O::CellSetRowSpan => ("rowspan", &["td", "th"], K::RowSpan),
        O::OListStart | O::OListSetStart => ("start", &["ol"], K::Signed(1)),
        O::OListReversed | O::OListSetReversed => ("reversed", &["ol"], K::Boolean),
        O::LIValue | O::LISetValue => ("value", &["li"], K::Signed(0)),
        O::DirectoryCompact | O::DirectoryCompactSet => ("compact", &["dir"], K::Boolean),
        O::FontColor | O::FontColorSet => ("color", &["font"], K::LegacyString),
        O::FontFace | O::FontFaceSet => ("face", &["font"], K::String),
        O::FontSize | O::FontSizeSet => ("size", &["font"], K::String),
        O::FrameSetCols | O::FrameSetColsSet => ("cols", &["frameset"], K::String),
        O::FrameSetRows | O::FrameSetRowsSet => ("rows", &["frameset"], K::String),
        O::FrameName | O::FrameNameSet => ("name", &["frame"], K::String),
        O::FrameScrolling | O::FrameScrollingSet => ("scrolling", &["frame"], K::String),
        O::FrameSrc | O::FrameSrcSet => ("src", &["frame"], K::UsvUrl),
        O::FrameBorder | O::FrameBorderSet => ("frameborder", &["frame"], K::String),
        O::FrameLongDesc | O::FrameLongDescSet => ("longdesc", &["frame"], K::UsvUrl),
        O::FrameNoResize | O::FrameNoResizeSet => ("noresize", &["frame"], K::Boolean),
        O::FrameMarginHeight | O::FrameMarginHeightSet => ("marginheight", &["frame"], K::LegacyString),
        O::FrameMarginWidth | O::FrameMarginWidthSet => ("marginwidth", &["frame"], K::LegacyString),
        _ => return None,
    };
    let setter = matches!(op, O::DirectoryCompactSet | O::FontColorSet | O::FontFaceSet | O::FontSizeSet
        | O::FrameSetColsSet | O::FrameSetRowsSet | O::FrameNameSet | O::FrameScrollingSet | O::FrameSrcSet
        | O::FrameBorderSet | O::FrameLongDescSet | O::FrameNoResizeSet | O::FrameMarginHeightSet
        | O::FrameMarginWidthSet | O::ElementSetHref | O::ElementSetTarget | O::ElementSetRel
        | O::ElementSetDownload | O::ElementSetSrc | O::ImageSetAlt | O::ImageSetWidth
        | O::ImageSetHeight | O::ImageSetLoading | O::ImageSetSrcset | O::ImageSetSizes
        | O::ImageSetDecoding | O::SourceSetType | O::SourceSetMedia | O::SourceSetSrcset
        | O::SourceSetSizes | O::InputSetType | O::ElementSetName
        | O::ElementSetDisabled | O::InputSetPlaceholder | O::ResourceSetType
        | O::ScriptSetAsync | O::ScriptSetDefer | O::LinkSetMedia | O::MetaSetContent
        | O::MetaSetHttpEquiv | O::MetaSetCharset | O::CellSetColSpan
        | O::CellSetRowSpan | O::OListSetStart | O::OListSetReversed | O::LISetValue
        | O::ElementSetRelList | O::OutputSetHtmlFor | O::IFrameSetSandbox | O::LinkSetSizes);
    Some(Spec { interface: None, attribute, tags, kind, setter })
}
pub(crate) fn handles(op: O) -> bool { spec(op).is_some() }
fn unsigned(number: f64) -> u32 {
    if number.is_finite() { number.trunc().rem_euclid(4_294_967_296.) as u32 } else { 0 }
}
fn input_type(value: &[u16]) -> Vec<u16> {
    const TYPES: &[&str] = &["hidden", "text", "search", "tel", "url", "email", "password", "date",
        "month", "week", "time", "datetime-local", "number", "range", "color", "checkbox",
        "radio", "file", "submit", "image", "reset", "button"];
    html_reflection::keyword(value, TYPES).unwrap_or_else(|| "text".encode_utf16().collect())
}
pub(crate) fn invoke<H: UrlReflection>(host: &mut H, op: O, receiver: H::Value,
    args: &[H::Value]) -> Result<H::Value, H::Error> {
    let spec = spec(op).expect("routed reflection operation");
    let node = host.node_brand(&receiver).ok_or_else(|| host.type_error("Expected an Element"))?;
    host.with_dom(|dom| match &dom.get(node).kind {
        NodeKind::Element { namespace: Namespace::Html, .. }
            if spec.interface.map_or_else(|| spec.tags.contains(&dom.local_name(node).unwrap()),
                |interface| crate::html_element_interface(dom.local_name(node).unwrap()) == interface) => Ok(()),
        _ => Err(DomError::TypeError),
    })?;
    if matches!(spec.kind, Kind::TokenList) {
        let slot = host.with_dom(|dom| dom.token_list(node, spec.attribute))?;
        let mut key = host.node_platform_key(node);
        key.slot = slot as u64;
        let list = host.wrap_platform_object(key, InterfaceBrand::DOM_TOKEN_LIST)?;
        return if spec.setter {
            crate::tokenlist::invoke(host, O::TokenListSetValue, list, args)
        } else { Ok(list) };
    }
    if spec.setter {
        let value = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
        if matches!(spec.kind, Kind::Boolean) {
            let enabled = host.to_boolean(value);
            // HTML-35: HTMLScriptElement.async's setter must clear the
            // "force async" flag on every call, per spec, before applying
            // the ordinary boolean-attribute set/remove below.
            if op == O::ScriptSetAsync {
                host.with_dom(|dom| { dom.set_force_async(node, false); Ok(()) })?;
            }
            host.with_dom(|dom| if enabled {
                html_reflection::set(dom, node, spec.attribute, Vec::new())
            } else { dom.remove_attribute(node, spec.attribute) })?;
        } else {
            let units = match spec.kind {
                Kind::Unsigned | Kind::ReflectedUnsigned | Kind::ColSpan | Kind::RowSpan | Kind::Signed(_) => {
                    let number = host.to_number(value)?;
                    let unsigned = unsigned(number);
                    let value = match spec.kind {
                        Kind::ReflectedUnsigned => (if unsigned <= i32::MAX as u32 { unsigned } else { 0 }).to_string(),
                        Kind::ColSpan => unsigned.clamp(1, 1000).to_string(),
                        Kind::RowSpan => unsigned.min(65534).to_string(),
                        Kind::Signed(_) => (unsigned as i32).to_string(),
                        _ => unsigned.to_string(),
                    };
                    value.encode_utf16().collect()
                }
                Kind::LegacyString if host.is_nullish(value) && !host.is_undefined(value) => Vec::new(),
                Kind::UsvUrl => String::from_utf16_lossy(&host.to_dom_string(value)?).encode_utf16().collect(),
                _ => host.to_dom_string(value)?,
            };
            host.with_dom(|dom| html_reflection::set(dom, node, spec.attribute, units))?;
        }
        return Ok(host.undefined());
    }
    let value = host.with_dom(|dom| Ok(dom.attribute_utf16(node, spec.attribute)))?;
    match spec.kind {
        // HTML-35: HTMLScriptElement.async reflects the "force async" flag,
        // not a plain boolean content attribute -- true if the flag is set
        // OR the async attribute is present (see init_force_async/
        // set_force_async in amazing-dom's namespace.rs).
        Kind::Boolean if op == O::ScriptAsync => {
            let force_async = host.with_dom(|dom| Ok(dom.force_async(node)))?;
            Ok(host.boolean(value.is_some() || force_async))
        }
        Kind::Boolean => Ok(host.boolean(value.is_some())),
        Kind::ReflectedUnsigned => Ok(host.number(value.as_deref().and_then(html_reflection::unsigned_long)
            .filter(|value| *value <= i32::MAX as u32).unwrap_or(0) as f64)),
        Kind::Unsigned => Ok(host.number(value.as_deref().and_then(html_reflection::unsigned_long).unwrap_or(0) as f64)),
        Kind::ColSpan => Ok(host.number(value.as_deref().and_then(html_reflection::unsigned_long)
            .unwrap_or(1).clamp(1, 1000) as f64)),
        Kind::RowSpan => Ok(host.number(value.as_deref().and_then(html_reflection::unsigned_long)
            .unwrap_or(1).min(65534) as f64)),
        Kind::Signed(default) => Ok(host.number(value.as_deref().and_then(html_reflection::signed_long)
            .unwrap_or(default) as f64)),
        Kind::Url | Kind::UsvUrl => {
            let Some(value) = value else { return host.string(&[]) };
            let value = if matches!(spec.kind, Kind::UsvUrl) { String::from_utf16_lossy(&value).encode_utf16().collect() } else { value };
            let resolved = host.resolve_url(&value)?;
            host.string(&resolved)
        }
        Kind::InputType => host.string(&input_type(value.as_deref().unwrap_or_default())),
        Kind::Loading => host.string(&html_reflection::keyword(value.as_deref().unwrap_or_default(),
            &["eager", "lazy"]).unwrap_or_else(|| "eager".encode_utf16().collect())),
        Kind::Decoding => host.string(&html_reflection::keyword(value.as_deref().unwrap_or_default(),
            &["sync", "async", "auto"]).unwrap_or_else(|| "auto".encode_utf16().collect())),
        Kind::String | Kind::LegacyString => host.string(&value.unwrap_or_default()),
        // Getter-only limiting, per the WebIDL reflection algorithm --
        // setting falls through to the plain-string `_` arm above.
        Kind::LimitedKnownValues(keywords) => host.string(&html_reflection::keyword(
            value.as_deref().unwrap_or_default(), keywords).unwrap_or_default()),
        Kind::TokenList => unreachable!(),
    }
}
