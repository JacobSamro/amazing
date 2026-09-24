//! HTML Location's live URL view and navigation operations.
use crate::host::{BindingHost, InterfaceBrand as B, Operation as O};
use amazing_dom::DomError;
use amazing_net::ParsedUrl;

pub(crate) fn handles(op: O) -> bool {
    matches!(op, O::LocationHref | O::LocationSetHref | O::LocationOrigin |
        O::LocationProtocol | O::LocationSetProtocol | O::LocationHost | O::LocationSetHost |
        O::LocationHostname | O::LocationSetHostname | O::LocationPort | O::LocationSetPort |
        O::LocationPathname | O::LocationSetPathname | O::LocationSearch | O::LocationSetSearch |
        O::LocationHash | O::LocationSetHash | O::LocationAssign | O::LocationReplace |
        O::LocationReload | O::LocationAncestorOrigins | O::LocationToString |
        O::DOMStringListLength | O::DOMStringListItem | O::DOMStringListContains)
}

fn syntax_error<H: BindingHost, T>(host: &mut H) -> Result<T, H::Error> {
    host.with_dom(|_| Err(DomError::SyntaxError))
}

fn string<H: BindingHost>(host: &mut H, value: &str) -> Result<H::Value, H::Error> {
    host.string(&value.encode_utf16().collect::<Vec<_>>())
}

fn required_usv<H: BindingHost>(host: &mut H, args: &[H::Value]) -> Result<String, H::Error> {
    let value = args.first().ok_or_else(|| host.type_error("Missing required argument"))?;
    Ok(String::from_utf16_lossy(&host.to_dom_string(value)?))
}

pub(crate) fn invoke<H: BindingHost>(host: &mut H, op: O, receiver: H::Value,
    args: &[H::Value]) -> Result<H::Value, H::Error> {
    if matches!(op, O::DOMStringListLength | O::DOMStringListItem | O::DOMStringListContains) {
        if host.platform_object(&receiver, B::DOM_STRING_LIST).is_none() {
            return Err(host.type_error("Expected a DOMStringList"));
        }
        return match op {
            O::DOMStringListLength => Ok(host.number(0.0)),
            O::DOMStringListItem => {
                let value = args.first().ok_or_else(|| host.type_error("Missing required argument"))?;
                host.to_number(value)?;
                Ok(host.null())
            }
            O::DOMStringListContains => {
                required_usv(host, args)?;
                Ok(host.boolean(false))
            }
            _ => unreachable!(),
        };
    }
    if host.platform_object(&receiver, B::LOCATION).is_none() {
        return Err(host.type_error("Expected a Location"));
    }
    if op == O::LocationAncestorOrigins { return host.location_ancestor_origins(); }
    if op == O::LocationReload {
        host.location_reload()?;
        return Ok(host.undefined());
    }
    let href = host.location_href()?;
    if matches!(op, O::LocationHref | O::LocationToString) { return string(host, &href); }
    let parsed = ParsedUrl::parse(&href).ok();
    if matches!(op, O::LocationAssign | O::LocationReplace | O::LocationSetHref) {
        let input = required_usv(host, args)?;
        let url = match ParsedUrl::parse_with_base(&input, parsed.as_ref()) {
            Ok(url) => url,
            Err(_) => return syntax_error(host),
        };
        host.location_navigate(&url.serialize(), op == O::LocationReplace)?;
        return Ok(host.undefined());
    }
    let Some(mut url) = parsed else {
        return if matches!(op, O::LocationOrigin) { string(host, "null") }
            else if matches!(op, O::LocationProtocol | O::LocationHost | O::LocationHostname |
                O::LocationPort | O::LocationPathname | O::LocationSearch | O::LocationHash) {
                string(host, "")
            } else { Ok(host.undefined()) };
    };
    let value = match op {
        O::LocationOrigin => url.origin().serialize(),
        O::LocationProtocol => format!("{}:", url.scheme()),
        O::LocationHost => url.host_and_port(),
        O::LocationHostname => url.hostname(),
        O::LocationPort => url.port().map(|port| port.to_string()).unwrap_or_default(),
        O::LocationPathname => url.pathname(),
        O::LocationSearch => url.query().map(|query| format!("?{query}")).unwrap_or_default(),
        O::LocationHash => url.fragment().map(|fragment| format!("#{fragment}")).unwrap_or_default(),
        _ => {
            let input = required_usv(host, args)?;
            match op {
                O::LocationSetProtocol => url.set_protocol(&input),
                O::LocationSetHost => url.set_host(&input),
                O::LocationSetHostname => url.set_hostname(&input),
                O::LocationSetPort => url.set_port(&input),
                O::LocationSetPathname => url.set_pathname(&input),
                O::LocationSetSearch => url.set_search(&input),
                O::LocationSetHash => url.set_hash(&input),
                _ => return Err(host.type_error("Unknown Location operation")),
            }
            host.location_navigate(&url.serialize(), false)?;
            return Ok(host.undefined());
        }
    };
    string(host, &value)
}
