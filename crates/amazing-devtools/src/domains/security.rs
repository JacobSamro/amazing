//! The `Security` domain: target security state and certificate handling.

use super::object;
use crate::host::SecurityState;
use crate::json::Json;
use crate::protocol::{CdpError, CdpRequest};
use crate::session::{DispatchOutcome, SessionState};

/// Handles a `Security.*` command.
pub fn dispatch(session: &mut SessionState, request: &CdpRequest) -> Result<DispatchOutcome, CdpError> {
    match request.method.as_str() {
        "Security.enable" => {
            session.enabled.security = true;
            let state = session.dev.security_state().map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty().with_events(vec![state_changed(&state)]))
        }
        "Security.disable" => {
            session.enabled.security = false;
            Ok(DispatchOutcome::empty())
        }
        "Security.setIgnoreCertificateErrors" => {
            let ignore = request.bool_param("ignore")?;
            session
                .dev
                .set_ignore_certificate_errors(ignore)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Security.handleCertificateError" => {
            let event_id = request
                .optional_i64("eventId")?
                .ok_or_else(|| CdpError::invalid_params("missing eventId"))?;
            let action = request.optional_i64("action")?.unwrap_or(0) as i32;
            session
                .dev
                .handle_certificate_error(event_id, action)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Security.setOverrideCertificateErrors" => {
            let _override = request.bool_param("override")?;
            Ok(DispatchOutcome::empty())
        }
        other => Err(CdpError::method_not_found(other)),
    }
}

/// Builds the `Security.securityStateChanged` event.
pub fn state_changed(state: &SecurityState) -> Json {
    crate::protocol::event(
        "Security.securityStateChanged",
        object(vec![
            ("securityState", Json::String(state.security_state.clone())),
            (
                "schemeIsCryptographic",
                Json::Bool(state.scheme_is_cryptographic),
            ),
            (
                "explanations",
                Json::Array(
                    state
                        .explanations
                        .iter()
                        .map(|text| Json::String(text.clone()))
                        .collect(),
                ),
            ),
            (
                "insecureContentStatus",
                object(vec![
                    ("blockedMixedContent", Json::Bool(false)),
                    ("containedMixedContent", Json::Bool(false)),
                    ("displayedMixedContent", Json::Bool(false)),
                    ("mixedContentType", Json::String("none".into())),
                    ("ranMixedContent", Json::Bool(false)),
                    ("ranContentWithCertErrors", Json::Bool(false)),
                    ("didBlockMixedContent", Json::Bool(false)),
                    ("ranInsecureContent", Json::Bool(false)),
                    (
                        "ranInsecureContentStyle",
                        Json::String(state.insecure_content_status.clone()),
                    ),
                ]),
            ),
        ]),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{DevToolsSession, HostError};
    use crate::session::SessionState;

    struct Secure;

    impl DevToolsSession for Secure {
        fn security_state(&mut self) -> Result<SecurityState, HostError> {
            Ok(SecurityState {
                security_state: "secure".into(),
                scheme_is_cryptographic: true,
                explanations: vec!["Confirmed by a trusted certificate".into()],
                insecure_content_status: "neutral".into(),
            })
        }
        fn set_ignore_certificate_errors(&mut self, _ignore: bool) -> Result<(), HostError> {
            Ok(())
        }
        fn handle_certificate_error(
            &mut self,
            _event_id: i64,
            _action: i32,
        ) -> Result<(), HostError> {
            Ok(())
        }
    }

    #[test]
    fn enable_emits_current_state() {
        let mut session = SessionState::new(
            "s",
            crate::host::TargetInfo::page("T1", "https://a/", "A"),
            Box::new(Secure),
        );
        let request = CdpRequest::parse(r#"{"id":1,"method":"Security.enable"}"#).unwrap();
        let outcome = dispatch(&mut session, &request).unwrap();
        assert!(session.enabled.security);
        let params = outcome.events[0].get("params").unwrap();
        assert_eq!(params.get("securityState").unwrap().as_str(), Some("secure"));
        assert_eq!(
            params.get("schemeIsCryptographic").unwrap().as_bool(),
            Some(true)
        );
    }
}
