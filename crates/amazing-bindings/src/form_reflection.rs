//! HTML form content-attribute reflection shared by the DOM-15 interfaces.
use crate::host::{BindingHost, FormAttribute as A, InterfaceBrand, Operation as O, UrlReflection};
use crate::html_reflection;
use amazing_dom::{DomError, Namespace, NodeKind};

#[derive(Clone, Copy)]
enum Kind {
    String, Boolean, Url, Unsigned(u32), NonNegative, Positive(u32),
    Keyword(&'static [&'static str], &'static str), Autocomplete,
    TokenList,
}
struct Spec { name: &'static str, tags: &'static [&'static str], kind: Kind }

fn spec(attribute: A) -> Spec {
    use Kind as K;
    let (name, tags, kind) = match attribute {
        A::Accept => ("accept", &["input"][..], K::String),
        A::Alpha => ("alpha", &["input"][..], K::Boolean),
        A::Alt => ("alt", &["input"][..], K::String),
        A::Autocomplete => ("autocomplete", &["input", "form", "select", "textarea"][..], K::Autocomplete),
        A::DirName => ("dirname", &["input", "textarea"][..], K::String),
        A::Disabled => ("disabled", &["input", "button", "select", "textarea", "option", "fieldset", "optgroup"][..], K::Boolean),
        A::FormAction => ("formaction", &["input", "button"][..], K::Url),
        A::FormEnctype => ("formenctype", &["input", "button"][..], K::Keyword(
            &["application/x-www-form-urlencoded", "multipart/form-data", "text/plain"], "application/x-www-form-urlencoded")),
        A::FormMethod => ("formmethod", &["input", "button"][..], K::Keyword(&["get", "post", "dialog"], "get")),
        A::FormNoValidate => ("formnovalidate", &["input", "button"][..], K::Boolean),
        A::FormTarget => ("formtarget", &["input", "button"][..], K::String),
        A::Height => ("height", &["input"][..], K::Unsigned(0)),
        A::Max => ("max", &["input"][..], K::String),
        A::MaxLength => ("maxlength", &["input", "textarea"][..], K::NonNegative),
        A::Min => ("min", &["input"][..], K::String),
        A::MinLength => ("minlength", &["input", "textarea"][..], K::NonNegative),
        A::Multiple => ("multiple", &["input", "select"][..], K::Boolean),
        A::Name => ("name", &["input", "button", "select", "textarea", "form", "output", "fieldset"][..], K::String),
        A::Pattern => ("pattern", &["input"][..], K::String),
        A::Placeholder => ("placeholder", &["input", "textarea"][..], K::String),
        A::ReadOnly => ("readonly", &["input", "textarea"][..], K::Boolean),
        A::Required => ("required", &["input", "select", "textarea"][..], K::Boolean),
        A::Size => ("size", &["input", "select"][..], K::Unsigned(0)),
        A::Src => ("src", &["input"][..], K::Url),
        A::Step => ("step", &["input"][..], K::String),
        A::Width => ("width", &["input"][..], K::Unsigned(0)),
        A::Action => ("action", &["form"][..], K::Url),
        A::AcceptCharset => ("accept-charset", &["form"][..], K::String),
        A::Enctype | A::Encoding => ("enctype", &["form"][..], K::Keyword(
            &["application/x-www-form-urlencoded", "multipart/form-data", "text/plain"], "application/x-www-form-urlencoded")),
        A::Method => ("method", &["form"][..], K::Keyword(&["get", "post", "dialog"], "get")),
        A::NoValidate => ("novalidate", &["form"][..], K::Boolean),
        A::Target => ("target", &["form"][..], K::String),
        A::Rel => ("rel", &["form"][..], K::String),
        A::RelList => ("rel", &["form"][..], K::TokenList),
        A::Cols => ("cols", &["textarea"][..], K::Positive(20)),
        A::Rows => ("rows", &["textarea"][..], K::Positive(2)),
        A::Wrap => ("wrap", &["textarea"][..], K::String),
        A::Label => ("label", &["option", "optgroup"][..], K::String),
        A::Value => ("value", &["button"][..], K::String),
        A::Command => ("command", &["button"][..], K::String),
        A::HtmlFor => ("for", &["label"][..], K::String),
    };
    Spec { name, tags, kind }
}

pub(crate) fn handles(op: O) -> bool {
    matches!(op, O::FormAttributeGet(_) | O::FormAttributeSet(_))
}

fn integer(number: f64) -> i32 {
    if number.is_finite() { number.trunc().rem_euclid(4_294_967_296.) as u32 as i32 } else { 0 }
}

pub(crate) fn invoke<H: BindingHost + UrlReflection>(host: &mut H, op: O,
    receiver: H::Value, args: &[H::Value]) -> Result<H::Value, H::Error> {
    let (attribute, setter) = match op {
        O::FormAttributeGet(attribute) => (attribute, false),
        O::FormAttributeSet(attribute) => (attribute, true),
        _ => unreachable!(),
    };
    let spec = spec(attribute);
    let node = host.node_brand(&receiver).ok_or_else(|| host.type_error("Expected a form element"))?;
    host.with_dom(|dom| match &dom.get(node).kind {
        NodeKind::Element { namespace: Namespace::Html, .. }
            if spec.tags.contains(&dom.local_name(node).unwrap()) => Ok(()),
        _ => Err(DomError::TypeError),
    })?;
    if matches!(spec.kind, Kind::TokenList) {
        let slot = host.with_dom(|dom| dom.token_list(node, spec.name))?;
        let mut key = host.node_platform_key(node);
        key.slot = slot as u64;
        return host.wrap_platform_object(key, InterfaceBrand::DOM_TOKEN_LIST);
    }
    if setter {
        let value = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
        if matches!(spec.kind, Kind::Boolean) {
            let enabled = host.to_boolean(value);
            host.with_dom(|dom| if enabled { html_reflection::set(dom, node, spec.name, Vec::new()) }
                else { dom.remove_attribute(node, spec.name) })?;
        } else {
            let units = match spec.kind {
                Kind::Unsigned(_) | Kind::NonNegative | Kind::Positive(_) => {
                    let integer = integer(host.to_number(value)?);
                    if (matches!(spec.kind, Kind::NonNegative) && integer < 0)
                        || (matches!(spec.kind, Kind::Positive(_)) && integer <= 0) {
                        return host.with_dom(|_| Err(DomError::IndexSizeError));
                    }
                    (integer as u32).to_string().encode_utf16().collect()
                }
                Kind::Url => String::from_utf16_lossy(&host.to_dom_string(value)?)
                    .encode_utf16().collect(),
                _ => host.to_dom_string(value)?,
            };
            host.with_dom(|dom| html_reflection::set(dom, node, spec.name, units))?;
        }
        return Ok(host.undefined());
    }
    let value = host.with_dom(|dom| Ok(dom.attribute_utf16(node, spec.name)))?;
    match spec.kind {
        Kind::Boolean => Ok(host.boolean(value.is_some())),
        Kind::Unsigned(default) => {
            let default = if attribute == A::Size && host.with_dom(|dom| Ok(dom.local_name(node) == Some("input")))? { 20 } else { default };
            Ok(host.number(value.as_deref().and_then(html_reflection::unsigned_long).unwrap_or(default) as f64))
        }
        Kind::NonNegative => Ok(host.number(value.as_deref().and_then(html_reflection::signed_long)
            .filter(|v| *v >= 0).unwrap_or(-1) as f64)),
        Kind::Positive(default) => Ok(host.number(value.as_deref().and_then(html_reflection::unsigned_long)
            .filter(|v| *v > 0).unwrap_or(default) as f64)),
        Kind::Url => {
            let resolved = host.resolve_url(value.as_deref().unwrap_or_default())?;
            host.string(&resolved)
        }
        Kind::Keyword(candidates, default) => host.string(&html_reflection::keyword(
            value.as_deref().unwrap_or_default(), candidates)
            .unwrap_or_else(|| default.encode_utf16().collect())),
        Kind::Autocomplete => {
            let units = value.as_deref().unwrap_or_default();
            let form = host.with_dom(|dom| Ok(dom.local_name(node) == Some("form")))?;
            let result = html_reflection::keyword(units, &["on", "off"])
                .unwrap_or_else(|| if form { "on".encode_utf16().collect() } else { units.to_vec() });
            host.string(&result)
        }
        Kind::String if attribute == A::Label && value.as_ref().is_none_or(Vec::is_empty) => {
            let text = host.with_dom(|dom| if dom.local_name(node) == Some("option") {
                dom.option_text(node)
            } else { Ok(Vec::new()) })?;
            host.string(&text)
        }
        Kind::String => host.string(&value.unwrap_or_default()),
        Kind::TokenList => unreachable!(),
    }
}
