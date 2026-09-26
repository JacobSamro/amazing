//! The `IndexedDB` domain: database listing and clearing.
//!
//! Chrome's full domain also browses object stores and rows
//! (`requestDatabase`/`requestData`); this crate covers the ticket's
//! `list`/`clear` scope (`requestDatabaseNames`, `clearObjectStore`,
//! `deleteDatabase`) and leaves row-level browsing for a later ticket.

use crate::json::Json;
use crate::protocol::{CdpError, CdpRequest};
use crate::session::{DispatchOutcome, SessionState};

/// Handles an `IndexedDB.*` command.
pub fn dispatch(session: &mut SessionState, request: &CdpRequest) -> Result<DispatchOutcome, CdpError> {
    match request.method.as_str() {
        "IndexedDB.enable" => {
            session.enabled.indexed_db = true;
            Ok(DispatchOutcome::empty())
        }
        "IndexedDB.disable" => {
            session.enabled.indexed_db = false;
            Ok(DispatchOutcome::empty())
        }
        "IndexedDB.requestDatabaseNames" => {
            let security_origin = security_origin(request)?;
            let names = session
                .dev
                .request_database_names(security_origin)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::result(super::object(vec![(
                "databaseNames",
                Json::Array(names.into_iter().map(Json::String).collect()),
            )])))
        }
        "IndexedDB.clearObjectStore" => {
            let security_origin = security_origin(request)?;
            let database_name = request.string_param("databaseName")?;
            let object_store_name = request.string_param("objectStoreName")?;
            session
                .dev
                .clear_object_store(security_origin, database_name, object_store_name)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "IndexedDB.deleteDatabase" => {
            let security_origin = security_origin(request)?;
            let database_name = request.string_param("databaseName")?;
            session
                .dev
                .delete_database(security_origin, database_name)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        other => Err(CdpError::method_not_found(other)),
    }
}

fn security_origin(request: &CdpRequest) -> Result<&str, CdpError> {
    request.string_param("securityOrigin")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{DevToolsSession, HostError};

    struct Scripted;

    impl DevToolsSession for Scripted {
        fn request_database_names(&mut self, security_origin: &str) -> Result<Vec<String>, HostError> {
            assert_eq!(security_origin, "https://a.example");
            Ok(vec!["db1".into(), "db2".into()])
        }
        fn delete_database(
            &mut self,
            _security_origin: &str,
            _database_name: &str,
        ) -> Result<(), HostError> {
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
    fn request_database_names_lists_the_databases() {
        let mut session = session();
        let request = CdpRequest::parse(
            r#"{"id":1,"method":"IndexedDB.requestDatabaseNames","params":{"securityOrigin":"https://a.example"}}"#,
        )
        .unwrap();
        let outcome = dispatch(&mut session, &request).unwrap();
        let names = outcome.result.get("databaseNames").unwrap().as_array().unwrap();
        assert_eq!(names.len(), 2);
        assert_eq!(names[0].as_str(), Some("db1"));
    }

    #[test]
    fn delete_database_calls_through() {
        let mut session = session();
        let request = CdpRequest::parse(
            r#"{"id":1,"method":"IndexedDB.deleteDatabase","params":{"securityOrigin":"https://a.example","databaseName":"db1"}}"#,
        )
        .unwrap();
        dispatch(&mut session, &request).unwrap();
    }
}
