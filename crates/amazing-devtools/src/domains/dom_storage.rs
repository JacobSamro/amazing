//! The `DOMStorage` domain: `localStorage`/`sessionStorage` items.

use super::object;
use crate::host::DomStorageId;
use crate::json::Json;
use crate::protocol::{CdpError, CdpRequest};
use crate::session::{DispatchOutcome, SessionState};

/// Handles a `DOMStorage.*` command.
pub fn dispatch(session: &mut SessionState, request: &CdpRequest) -> Result<DispatchOutcome, CdpError> {
    match request.method.as_str() {
        "DOMStorage.enable" => {
            session.enabled.dom_storage = true;
            Ok(DispatchOutcome::empty())
        }
        "DOMStorage.disable" => {
            session.enabled.dom_storage = false;
            Ok(DispatchOutcome::empty())
        }
        "DOMStorage.getDOMStorageItems" => {
            let storage_id = storage_id(request)?;
            let items = session
                .dev
                .get_dom_storage_items(&storage_id)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::result(object(vec![(
                "entries",
                Json::Array(
                    items
                        .into_iter()
                        .map(|(key, value)| Json::Array(vec![Json::String(key), Json::String(value)]))
                        .collect(),
                ),
            )])))
        }
        "DOMStorage.setDOMStorageItem" => {
            let storage_id = storage_id(request)?;
            let key = request.string_param("key")?;
            let value = request.string_param("value")?;
            session
                .dev
                .set_dom_storage_item(&storage_id, key, value)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "DOMStorage.removeDOMStorageItem" => {
            let storage_id = storage_id(request)?;
            let key = request.string_param("key")?;
            session
                .dev
                .remove_dom_storage_item(&storage_id, key)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "DOMStorage.clear" => {
            let storage_id = storage_id(request)?;
            session
                .dev
                .clear_dom_storage(&storage_id)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        other => Err(CdpError::method_not_found(other)),
    }
}

/// Parses a `DOMStorage.StorageId` parameter.
fn storage_id(request: &CdpRequest) -> Result<DomStorageId, CdpError> {
    let value = request
        .params
        .get("storageId")
        .ok_or_else(|| CdpError::invalid_params("missing storageId"))?;
    let security_origin = value
        .get("securityOrigin")
        .and_then(Json::as_str)
        .ok_or_else(|| CdpError::invalid_params("storageId missing securityOrigin"))?
        .to_string();
    let is_local_storage = value
        .get("isLocalStorage")
        .and_then(Json::as_bool)
        .ok_or_else(|| CdpError::invalid_params("storageId missing isLocalStorage"))?;
    Ok(DomStorageId {
        security_origin,
        is_local_storage,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{DevToolsSession, HostError};

    struct Scripted;

    impl DevToolsSession for Scripted {
        fn get_dom_storage_items(
            &mut self,
            storage_id: &DomStorageId,
        ) -> Result<Vec<(String, String)>, HostError> {
            assert_eq!(storage_id.security_origin, "https://a.example");
            assert!(storage_id.is_local_storage);
            Ok(vec![("theme".into(), "dark".into())])
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
    fn get_dom_storage_items_parses_the_storage_id() {
        let mut session = session();
        let request = CdpRequest::parse(
            r#"{"id":1,"method":"DOMStorage.getDOMStorageItems","params":{"storageId":{"securityOrigin":"https://a.example","isLocalStorage":true}}}"#,
        )
        .unwrap();
        let outcome = dispatch(&mut session, &request).unwrap();
        let entries = outcome.result.get("entries").unwrap().as_array().unwrap();
        assert_eq!(entries[0].as_array().unwrap()[0].as_str(), Some("theme"));
        assert_eq!(entries[0].as_array().unwrap()[1].as_str(), Some("dark"));
    }

    #[test]
    fn missing_storage_id_is_invalid_params() {
        let mut session = session();
        let request =
            CdpRequest::parse(r#"{"id":1,"method":"DOMStorage.getDOMStorageItems"}"#).unwrap();
        let error = dispatch(&mut session, &request).unwrap_err();
        assert_eq!(error.code, crate::protocol::INVALID_PARAMS);
    }
}
