//! HTMLHyperlinkElementUtils component accessors over the existing URL model.
use crate::host::{Operation as O, UrlReflection};
use amazing_dom::{DomError, Namespace, NodeKind};
use amazing_net::ParsedUrl;

pub(crate) fn handles(op: O) -> bool {
    matches!(op, O::HyperlinkOrigin | O::HyperlinkProtocol | O::HyperlinkSetProtocol
        | O::HyperlinkUsername | O::HyperlinkSetUsername | O::HyperlinkPassword | O::HyperlinkSetPassword
        | O::HyperlinkHost | O::HyperlinkSetHost | O::HyperlinkHostname | O::HyperlinkSetHostname
        | O::HyperlinkPort | O::HyperlinkSetPort | O::HyperlinkPathname | O::HyperlinkSetPathname
        | O::HyperlinkSearch | O::HyperlinkSetSearch | O::HyperlinkHash | O::HyperlinkSetHash)
}

pub(crate) fn invoke<H: UrlReflection>(host: &mut H, op: O, receiver: H::Value,
    args: &[H::Value]) -> Result<H::Value, H::Error> {
    let node = host.node_brand(&receiver).ok_or_else(|| host.type_error("Expected a hyperlink element"))?;
    host.with_dom(|dom| match &dom.get(node).kind {
        NodeKind::Element { name, namespace: Namespace::Html, .. } if matches!(name.as_str(), "a" | "area") => Ok(()),
        _ => Err(DomError::TypeError),
    })?;
    let setter = matches!(op, O::HyperlinkSetProtocol | O::HyperlinkSetUsername | O::HyperlinkSetPassword
        | O::HyperlinkSetHost | O::HyperlinkSetHostname | O::HyperlinkSetPort
        | O::HyperlinkSetPathname | O::HyperlinkSetSearch | O::HyperlinkSetHash);
    // WebIDL conversion can mutate the element/document; reinitialize afterward.
    let input = if setter {
        let arg = args.first().ok_or_else(|| host.type_error("Missing required argument"))?;
        Some(String::from_utf16_lossy(&host.to_dom_string(arg)?))
    } else { None };
    let href = host.with_dom(|dom| Ok(dom.attribute_utf16(node, "href")))?;
    let url = match href {
        Some(href) => {
            let resolved = host.resolve_element_url(node, &href)?;
            ParsedUrl::parse(&String::from_utf16_lossy(&resolved)).ok()
        }
        None => None,
    };
    let Some(mut url) = url else {
        return if setter { Ok(host.undefined()) }
            else { host.string(&if op == O::HyperlinkProtocol { vec![58] } else { vec![] }) };
    };
    let value = match op {
        O::HyperlinkOrigin => url.origin().serialize(),
        O::HyperlinkProtocol => format!("{}:", url.scheme()),
        O::HyperlinkUsername => url.username().into(), O::HyperlinkPassword => url.password().into(),
        O::HyperlinkHost => url.host_and_port(), O::HyperlinkHostname => url.hostname(),
        O::HyperlinkPort => url.port().map(|port| port.to_string()).unwrap_or_default(),
        O::HyperlinkPathname => url.pathname(),
        O::HyperlinkSearch => url.query().filter(|query| !query.is_empty()).map(|query| format!("?{query}")).unwrap_or_default(),
        O::HyperlinkHash => url.fragment().filter(|hash| !hash.is_empty()).map(|hash| format!("#{hash}")).unwrap_or_default(),
        _ => {
            let input = input.as_deref().expect("setter conversion");
            if matches!(op, O::HyperlinkSetHost | O::HyperlinkSetHostname | O::HyperlinkSetPathname)
                && url.has_opaque_path() {
                return Ok(host.undefined());
            }
            if matches!(op, O::HyperlinkSetUsername | O::HyperlinkSetPassword | O::HyperlinkSetPort)
                && (url.host().is_none() || url.hostname().is_empty() || url.scheme() == "file") {
                return Ok(host.undefined());
            }
            match op {
                O::HyperlinkSetProtocol => url.set_protocol(input),
                O::HyperlinkSetUsername => url.set_username(input),
                O::HyperlinkSetPassword => url.set_password(input),
                O::HyperlinkSetHost => url.set_host(input),
                O::HyperlinkSetHostname => url.set_hostname(input),
                O::HyperlinkSetPort => url.set_port(input),
                O::HyperlinkSetPathname => url.set_pathname(input),
                O::HyperlinkSetSearch => url.set_search(input),
                O::HyperlinkSetHash => url.set_hash(input),
                _ => unreachable!(),
            }
            let href = url.serialize().encode_utf16().collect();
            host.with_dom(|dom| crate::html_reflection::set(dom, node, "href", href))?;
            return Ok(host.undefined());
        }
    };
    host.string(&value.encode_utf16().collect::<Vec<_>>())
}
