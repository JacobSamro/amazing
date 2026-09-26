//! The `Storage` domain: cookies (`getCookies`/`setCookies`/`clearCookies`).
//!
//! `localStorage`/`sessionStorage` are the separate `DOMStorage` domain
//! ([`crate::domains::dom_storage`]) and IndexedDB is
//! [`crate::domains::indexed_db`], matching Chrome's own domain split; this
//! ticket groups all three under one "Storage" heading in the decision map
//! since they share one seam batch and one `list`/`clear` scope.

use super::object;
use crate::host::Cookie;
use crate::json::Json;
use crate::protocol::{CdpError, CdpRequest};
use crate::session::{DispatchOutcome, SessionState};

/// Handles a `Storage.*` command.
pub fn dispatch(session: &mut SessionState, request: &CdpRequest) -> Result<DispatchOutcome, CdpError> {
    match request.method.as_str() {
        "Storage.getCookies" => {
            let cookies = session.dev.get_cookies().map_err(CdpError::from)?;
            Ok(DispatchOutcome::result(object(vec![(
                "cookies",
                Json::Array(cookies.iter().map(cookie_json).collect()),
            )])))
        }
        "Storage.setCookies" => {
            let cookies = cookie_array(request)?;
            session.dev.set_cookies(&cookies).map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Storage.clearCookies" => {
            session.dev.clear_cookies().map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        other => Err(CdpError::method_not_found(other)),
    }
}

/// Serializes a `Network.Cookie`.
fn cookie_json(cookie: &Cookie) -> Json {
    object(vec![
        ("name", Json::String(cookie.name.clone())),
        ("value", Json::String(cookie.value.clone())),
        ("domain", Json::String(cookie.domain.clone())),
        ("path", Json::String(cookie.path.clone())),
        ("expires", Json::number(cookie.expires)),
        ("size", Json::number((cookie.name.len() + cookie.value.len()) as f64)),
        ("httpOnly", Json::Bool(cookie.http_only)),
        ("secure", Json::Bool(cookie.secure)),
        ("session", Json::Bool(cookie.expires < 0.0)),
        (
            "sameSite",
            if cookie.same_site.is_empty() {
                Json::Null
            } else {
                Json::String(cookie.same_site.clone())
            },
        ),
    ])
}

fn cookie_array(request: &CdpRequest) -> Result<Vec<Cookie>, CdpError> {
    let items = match request.params.get("cookies") {
        Some(Json::Array(items)) => items,
        _ => return Err(CdpError::invalid_params("cookies must be an array")),
    };
    items.iter().map(cookie_from_json).collect()
}

fn cookie_from_json(value: &Json) -> Result<Cookie, CdpError> {
    let name = value
        .get("name")
        .and_then(Json::as_str)
        .ok_or_else(|| CdpError::invalid_params("cookie missing name"))?
        .to_string();
    let cookie_value = value
        .get("value")
        .and_then(Json::as_str)
        .ok_or_else(|| CdpError::invalid_params("cookie missing value"))?
        .to_string();
    let domain = value
        .get("domain")
        .and_then(Json::as_str)
        .unwrap_or_default()
        .to_string();
    let path = value
        .get("path")
        .and_then(Json::as_str)
        .unwrap_or("/")
        .to_string();
    let expires = value.get("expires").and_then(Json::as_f64).unwrap_or(-1.0);
    let http_only = value.get("httpOnly").and_then(Json::as_bool).unwrap_or(false);
    let secure = value.get("secure").and_then(Json::as_bool).unwrap_or(false);
    let same_site = value
        .get("sameSite")
        .and_then(Json::as_str)
        .unwrap_or_default()
        .to_string();
    Ok(Cookie {
        name,
        value: cookie_value,
        domain,
        path,
        expires,
        http_only,
        secure,
        same_site,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{DevToolsSession, HostError};

    struct Scripted;

    impl DevToolsSession for Scripted {
        fn get_cookies(&mut self) -> Result<Vec<Cookie>, HostError> {
            Ok(vec![Cookie {
                name: "sid".into(),
                value: "abc".into(),
                domain: "a.example".into(),
                path: "/".into(),
                expires: -1.0,
                http_only: true,
                secure: true,
                same_site: "Strict".into(),
            }])
        }
        fn set_cookies(&mut self, _cookies: &[Cookie]) -> Result<(), HostError> {
            Ok(())
        }
    }

    fn session() -> SessionState {
        SessionState::new(
            "s",
            crate::host::TargetInfo::page("T1", "https://a/", "A"),
            Box::new(Scripted),
        )
    }

    #[test]
    fn get_cookies_serializes_flags_and_same_site() {
        let mut session = session();
        let request = CdpRequest::parse(r#"{"id":1,"method":"Storage.getCookies"}"#).unwrap();
        let outcome = dispatch(&mut session, &request).unwrap();
        let cookies = outcome.result.get("cookies").unwrap().as_array().unwrap();
        assert_eq!(cookies[0].get("name").unwrap().as_str(), Some("sid"));
        assert_eq!(cookies[0].get("httpOnly").unwrap().as_bool(), Some(true));
        assert_eq!(cookies[0].get("sameSite").unwrap().as_str(), Some("Strict"));
        assert_eq!(cookies[0].get("session").unwrap().as_bool(), Some(true));
    }

    #[test]
    fn set_cookies_parses_the_array() {
        let mut session = session();
        let request = CdpRequest::parse(
            r#"{"id":1,"method":"Storage.setCookies","params":{"cookies":[{"name":"a","value":"b"}]}}"#,
        )
        .unwrap();
        dispatch(&mut session, &request).unwrap();
    }
}
