//! The `Target`/`Browser` domain serialization.
//!
//! Routing for these lives in [`crate::server`] because they act on the host
//! and can create further sessions; this module only builds their JSON.

use super::object;
use crate::host::{HostInfo, TargetInfo};
use crate::json::Json;

/// Serializes a CDP `Target.TargetInfo`.
pub fn target_info(target: &TargetInfo) -> Json {
    let mut fields = vec![
        ("targetId", Json::String(target.id.clone())),
        ("type", Json::String(target.target_type.clone())),
        ("title", Json::String(target.title.clone())),
        ("url", Json::String(target.url.clone())),
        ("attached", Json::Bool(target.attached)),
        ("canAccessOpener", Json::Bool(false)),
    ];
    if let Some(opener) = &target.opener_id {
        fields.push(("openerId", Json::String(opener.clone())));
    }
    if let Some(context) = &target.browser_context_id {
        fields.push(("browserContextId", Json::String(context.clone())));
    }
    object(fields)
}

/// Serializes a CDP `Browser.getVersion` result.
pub fn browser_version(info: &HostInfo) -> Json {
    object(vec![
        ("protocolVersion", Json::String(info.protocol_version.clone())),
        ("product", Json::String(info.product.clone())),
        ("revision", Json::String(info.revision.clone())),
        ("userAgent", Json::String(info.user_agent.clone())),
        ("jsVersion", Json::String(info.js_version.clone())),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_info_serializes_optionals() {
        let mut target = TargetInfo::page("T1", "https://a/", "A");
        target.opener_id = Some("T0".into());
        target.attached = true;
        let json = target_info(&target);
        assert_eq!(json.get("targetId").unwrap().as_str(), Some("T1"));
        assert_eq!(json.get("openerId").unwrap().as_str(), Some("T0"));
        assert_eq!(json.get("canAccessOpener").unwrap().as_bool(), Some(false));
    }

    #[test]
    fn browser_version_shape() {
        let info = HostInfo::default();
        let json = browser_version(&info);
        assert_eq!(json.get("protocolVersion").unwrap().as_str(), Some("1.3"));
        assert!(json.get("userAgent").unwrap().as_str().unwrap().contains("Amazing"));
    }
}
