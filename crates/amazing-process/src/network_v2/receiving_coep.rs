//! Serialize only COEP authority captured from the actual receiving environment.
use super::{invalid, origin};
use crate::protocol::NetworkContext;
use amazing_fetch::{ClientContext, EmbedderPolicyContainer, Origin};
use amazing_ipc::Error;

/// Apply the captured enforced COEP value to an already-authored wire request.
/// The full container remains with its original browser owner for report-only
/// checks and endpoint custody. This does not reconstruct CSP, sites, or purpose.
pub fn apply_receiving_coep(wire: &mut NetworkContext, client: &ClientContext,
    native_origin: &Origin, container: &EmbedderPolicyContainer) -> Result<(), Error> {
    if origin(&wire.origin)? != *native_origin {
        return Err(invalid("receiving origin mismatch"));
    }
    if client.security.embedder_policy != container.value {
        return Err(invalid("receiving embedder policy mismatch"));
    }
    wire.embedder_policy = container.value.as_str().into();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{WireCookieSite, WireOrigin};
    use amazing_fetch::EmbedderPolicy;
    fn wire(native: &Origin) -> NetworkContext {
        NetworkContext { origin:super::super::origin_to_wire(native),
            cookie_site:WireCookieSite::NoClient(()),top_level_origin:None,
            ancestor_origins:vec![],sandbox_flags:0,now_unix:0,
            restricts_mixed_content:false,image_set_initiator:false,
            top_level_navigation:false,embedder_policy:"uninitialized".into(),
            no_cors_media_state:"none".into(),csp_headers:vec![] }
    }
    #[test]
    fn captured_policy_serialization_preserves_opaque_identity_and_rejects_mismatch() {
        let native=Origin::opaque_with_id(91);
        for value in [EmbedderPolicy::UnsafeNone,EmbedderPolicy::RequireCorp,EmbedderPolicy::Credentialless] {
            let mut client=ClientContext::default();
            client.security.embedder_policy=value;
            let container=EmbedderPolicyContainer { value, ..Default::default() };
            let mut encoded=wire(&native);
            apply_receiving_coep(&mut encoded,&client,&native,&container).unwrap();
            assert_eq!(encoded.embedder_policy,value.as_str());
            assert_eq!(encoded.origin,WireOrigin::Opaque(91));
            let before=encoded.clone();
            assert!(apply_receiving_coep(&mut encoded,&client,&Origin::opaque_with_id(92),&container).is_err());
            assert_eq!(encoded,before);
            let wrong=EmbedderPolicyContainer { value:if value==EmbedderPolicy::UnsafeNone {
                EmbedderPolicy::Credentialless } else { EmbedderPolicy::UnsafeNone }, ..Default::default() };
            assert!(apply_receiving_coep(&mut encoded,&client,&native,&wrong).is_err());
            assert_eq!(encoded,before);
        }
    }
}
