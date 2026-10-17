//! WHATWG Streams (`ReadableStream`, `WritableStream`, `TransformStream`,
//! queuing strategies, `TextEncoderStream`/`TextDecoderStream`) and the
//! Request/Response Body-mixin glue. The stream machinery itself is a
//! JavaScript prelude (`runtime_host_streams/streams_*.js`) evaluated once per
//! realm; this adapter only supplies the native `take` hook and forwards the
//! Fetch bindings' stream hooks to the prelude's `hooks` object.
use super::*;
use amazing_bindings::fetch_interfaces::{FetchHost, FetchInterface, FetchObject};

const PRELUDE: [&str; 5] = [
    include_str!("runtime_host_streams/streams_core.js"),
    include_str!("runtime_host_streams/streams_byte.js"),
    include_str!("runtime_host_streams/streams_writable.js"),
    include_str!("runtime_host_streams/streams_transform.js"),
    include_str!("runtime_host_streams/streams_glue.js"),
];

#[derive(Clone, Copy, Debug)]
pub(crate) enum StreamsOp {
    /// `native.take(requestOrResponse)`: consume the Rust-side body and return
    /// its bytes as a `Uint8Array`.
    Take,
}

impl RuntimeBindingHost<'_> {
    pub(super) fn install_streams(&mut self) -> Result<(), Throw> {
        let realm = self.js.binding_current_realm();
        let source = format!("(function(native){{\n{}\n}})", PRELUDE.concat());
        let function = self.js.binding_eval_in_realm(realm, &source)?;
        let object_proto = self.js.binding_object_prototype();
        let native_object = self.js.binding_create_object(Some(&object_proto))?;
        let take = self.native("take", 1, NativeEntry::Streams(StreamsOp::Take), HostCallPolicy::CallOnly);
        self.define_data_property(&native_object, "take", take, true, true, true)?;
        let existing = self.state.borrow().streams_hooks.values().next().cloned();
        if let Some(existing) = existing {
            let brands = self.js.binding_get_property(&existing, "brands")?;
            self.define_data_property(&native_object, "brands", brands, true, true, true)?;
        }
        let hooks = binding_call(self.js, &function, &JsValue::Undefined, &[native_object])?;
        self.state.borrow_mut().streams_hooks.insert(realm, hooks);
        Ok(())
    }

    pub(super) fn streams_hook(&mut self, name: &str, args: &[JsValue]) -> Result<Option<JsValue>, Throw> {
        let realm = self.js.binding_current_realm();
        let Some(hooks) = self.state.borrow().streams_hooks.get(&realm).cloned() else {
            return Ok(None);
        };
        let function = self.js.binding_get_property(&hooks, name)?;
        binding_call(self.js, &function, &hooks, args).map(Some)
    }

    pub(super) fn invoke_streams(&mut self, op: StreamsOp, args: &[JsValue]) -> Result<JsValue, Throw> {
        match op {
            StreamsOp::Take => {
                let target = args.first().cloned().ok_or_else(|| Throw::type_error("take requires a body owner"))?;
                let record = self
                    .fetch_record(&target, FetchInterface::Response)
                    .or_else(|_| self.fetch_record(&target, FetchInterface::Request))?;
                let bytes = {
                    let mut object = record.borrow_mut();
                    let body = match &mut *object {
                        FetchObject::Request(request, _) => &mut request.body,
                        FetchObject::Response(response, _) => &mut response.body,
                        _ => return Err(Throw::type_error("Expected Body")),
                    };
                    body.bytes().map_err(|error| Throw::type_error(format!("{error:?}")))?
                };
                self.fetch_uint8_array(bytes)
            }
        }
    }
}
