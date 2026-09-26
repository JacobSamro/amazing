//! The CDP domains: serialization helpers plus one module per domain.
//!
//! `Target`/`Browser` live in [`crate::server`] because they act on the host
//! rather than on a single attached page; the page-bound domains are dispatched
//! from [`crate::session`].

pub mod accessibility;
pub mod css;
pub mod debugger;
pub mod dom;
pub mod dom_snapshot;
pub mod dom_storage;
pub mod emulation;
pub mod fetch;
pub mod heap_profiler;
pub mod indexed_db;
pub mod input;
pub mod log;
pub mod network;
pub mod overlay;
pub mod page;
pub mod performance;
pub mod profiler;
pub mod runtime;
pub mod security;
pub mod storage;
pub mod target;
pub mod tracing;

use crate::host::{
    ExceptionDetails, FrameInfo, FrameTree, ObjectPreview, PropertyDescriptor, PropertyPreview,
    RemoteObject, StyleOrigin,
};
use crate::json::Json;

/// Builds a JSON object from ordered fields.
pub(crate) fn object(fields: Vec<(&str, Json)>) -> Json {
    Json::Object(
        fields
            .into_iter()
            .map(|(name, value)| (name.to_string(), value))
            .collect(),
    )
}

/// An empty JSON array.
pub(crate) fn empty_array() -> Json {
    Json::Array(Vec::new())
}

/// Serializes a CDP `Headers` map.
pub(crate) fn headers(headers: &[(String, String)]) -> Json {
    Json::Object(
        headers
            .iter()
            .map(|(name, value)| (name.clone(), Json::String(value.clone())))
            .collect(),
    )
}

/// Serializes a CDP `Runtime.RemoteObject`.
pub(crate) fn remote_object(value: &RemoteObject) -> Json {
    let mut fields = vec![("type", Json::String(value.object_type.clone()))];
    if let Some(subtype) = &value.subtype {
        fields.push(("subtype", Json::String(subtype.clone())));
    }
    if let Some(class_name) = &value.class_name {
        fields.push(("className", Json::String(class_name.clone())));
    }
    if let Some(inner) = &value.value {
        fields.push(("value", inner.clone()));
    }
    if let Some(unserializable) = &value.unserializable_value {
        fields.push(("unserializableValue", Json::String(unserializable.clone())));
    }
    if let Some(description) = &value.description {
        fields.push(("description", Json::String(description.clone())));
    }
    if let Some(object_id) = &value.object_id {
        fields.push(("objectId", Json::String(object_id.clone())));
    }
    if let Some(preview) = &value.preview {
        fields.push(("preview", object_preview(preview)));
    }
    object(fields)
}

/// Serializes a CDP `Runtime.ObjectPreview` (CDP-24).
pub(crate) fn object_preview(preview: &ObjectPreview) -> Json {
    let mut fields = vec![("type", Json::String(preview.object_type.clone()))];
    if let Some(subtype) = &preview.subtype {
        fields.push(("subtype", Json::String(subtype.clone())));
    }
    if let Some(description) = &preview.description {
        fields.push(("description", Json::String(description.clone())));
    }
    fields.push(("overflow", Json::Bool(preview.overflow)));
    fields.push((
        "properties",
        Json::Array(preview.properties.iter().map(property_preview).collect()),
    ));
    object(fields)
}

/// Serializes one `Runtime.PropertyPreview` entry.
fn property_preview(preview: &PropertyPreview) -> Json {
    let mut fields = vec![
        ("name", Json::String(preview.name.clone())),
        ("type", Json::String(preview.object_type.clone())),
    ];
    if let Some(value) = &preview.value {
        fields.push(("value", Json::String(value.clone())));
    }
    object(fields)
}

/// Serializes a CDP `Runtime.ExceptionDetails`.
pub(crate) fn exception_details(details: &ExceptionDetails) -> Json {
    let mut fields = vec![
        ("text", Json::String(details.text.clone())),
        ("lineNumber", Json::number(details.line_number as f64)),
        ("columnNumber", Json::number(details.column_number as f64)),
    ];
    if let Some(url) = &details.url {
        fields.push(("url", Json::String(url.clone())));
    }
    if let Some(exception) = &details.exception {
        fields.push(("exception", remote_object(exception)));
    }
    if let Some(stack) = &details.stack_trace {
        fields.push(("stackTrace", Json::String(stack.clone())));
    }
    object(fields)
}

/// Serializes a CDP `Runtime.PropertyDescriptor`.
pub(crate) fn property_descriptor(property: &PropertyDescriptor) -> Json {
    object(vec![
        ("name", Json::String(property.name.clone())),
        ("value", remote_object(&property.value)),
        ("writable", Json::Bool(property.writable)),
        ("configurable", Json::Bool(property.configurable)),
        ("enumerable", Json::Bool(property.enumerable)),
        ("isOwn", Json::Bool(property.is_own)),
    ])
}

/// Serializes a CDP `Page.Frame`.
pub(crate) fn frame(frame: &FrameInfo) -> Json {
    let mut fields = vec![
        ("id", Json::String(frame.id.clone())),
        ("loaderId", Json::String(String::new())),
        ("url", Json::String(frame.url.clone())),
        ("domainAndRegistry", Json::String(String::new())),
        ("securityOrigin", Json::String(frame.security_origin.clone())),
        ("mimeType", Json::String(frame.mime_type.clone())),
        ("secureContextType", Json::String("Secure".into())),
        (
            "crossOriginIsolatedContextType",
            Json::String("NotIsolated".into()),
        ),
        ("gatedAPIFeatures", empty_array()),
    ];
    if let Some(parent) = &frame.parent_id {
        fields.push(("parentId", Json::String(parent.clone())));
    }
    object(fields)
}

/// Serializes a CDP `Page.FrameTree`.
pub(crate) fn frame_tree(tree: &FrameTree) -> Json {
    object(vec![
        ("frame", frame(&tree.frame)),
        (
            "childFrames",
            Json::Array(tree.child_frames.iter().map(frame_tree).collect()),
        ),
    ])
}

/// Encodes resource bytes as CDP text or base64, returning whether the body is
/// base64-encoded.
pub(crate) fn encode_body(content: crate::host::ResourceContent) -> (String, bool) {
    match String::from_utf8(content.bytes) {
        Ok(text) => (text, false),
        Err(error) => (crate::base64::encode(error.as_bytes()), true),
    }
}

/// A CSS style origin string.
pub(crate) fn style_origin(origin: StyleOrigin) -> &'static str {
    match origin {
        StyleOrigin::UserAgent => "user-agent",
        StyleOrigin::User => "user",
        StyleOrigin::Author => "author",
        StyleOrigin::Inspector => "inspector",
    }
}
