//! Fetch and XHR WebIDL surface. The driver owns JS values and the network;
//! this module owns conversion, interface shape, and native algorithm dispatch.
use crate::host::{BindingHost, HostCallPolicy, InterfaceBrand, PlatformKey};
use amazing_dom::abort::AbortSignalId;
use amazing_dom::NodeKind;
use amazing_fetch::{
    HttpUrl,
    api::{
        ApiError, ApiHeaders, ApiRequest, ApiRequestInit, ApiRequestInput, ApiResponse,
        ApiResponseInit, BodyInput, JsonValue,
    },
    xhr::{Xhr, XhrError, XhrEvent, XhrResponseType, XhrResponseValue, XhrTextDecoder},
};
use std::{cell::RefCell, rc::Rc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FetchInterface {
    Headers,
    Request,
    Response,
    XMLHttpRequest,
    XMLHttpRequestUpload,
    ProgressEvent,
    HeadersIterator,
}
impl FetchInterface {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Headers => "Headers",
            Self::Request => "Request",
            Self::Response => "Response",
            Self::XMLHttpRequest => "XMLHttpRequest",
            Self::XMLHttpRequestUpload => "XMLHttpRequestUpload",
            Self::ProgressEvent => "ProgressEvent",
            Self::HeadersIterator => "Headers Iterator",
        }
    }
    pub const fn brand(self) -> InterfaceBrand {
        InterfaceBrand(match self {
            Self::Headers => 100,
            Self::Request => 101,
            Self::Response => 102,
            Self::XMLHttpRequest => 103,
            Self::XMLHttpRequestUpload => 104,
            Self::ProgressEvent => 105,
            Self::HeadersIterator => 106,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FetchOperation {
    Fetch,
    HeadersConstructor,
    HeadersAppend,
    HeadersSet,
    HeadersDelete,
    HeadersGet,
    HeadersHas,
    HeadersGetSetCookie,
    HeadersForEach,
    HeadersKeys,
    HeadersValues,
    HeadersEntries,
    RequestConstructor,
    RequestClone,
    RequestMethod,
    RequestUrl,
    RequestHeaders,
    RequestMode,
    RequestCredentials,
    RequestCache,
    RequestRedirect,
    RequestReferrer,
    RequestReferrerPolicy,
    RequestIntegrity,
    RequestKeepalive,
    RequestBody,
    RequestBodyUsed,
    RequestSignal,
    ResponseConstructor,
    ResponseError,
    ResponseRedirect,
    ResponseClone,
    ResponseType,
    ResponseUrl,
    ResponseRedirected,
    ResponseStatus,
    ResponseStatusText,
    ResponseOk,
    ResponseHeaders,
    ResponseBody,
    ResponseBodyUsed,
    BodyText,
    BodyJson,
    BodyBlob,
    BodyArrayBuffer,
    BodyFormData,
    BodyBytes,
    XhrConstructor,
    XhrOpen,
    XhrSetRequestHeader,
    XhrSend,
    XhrAbort,
    XhrGetResponseHeader,
    XhrGetAllResponseHeaders,
    XhrOverrideMimeType,
    XhrReadyState,
    XhrStatus,
    XhrStatusText,
    XhrResponseUrl,
    XhrResponseType,
    XhrSetResponseType,
    XhrResponseText,
    XhrResponse,
    XhrResponseXml,
    XhrTimeout,
    XhrSetTimeout,
    XhrWithCredentials,
    XhrSetWithCredentials,
    XhrUpload,
    XhrAddEventListener,
    XhrRemoveEventListener,
    ProgressEventConstructor,
    ProgressEventType,
    ProgressEventLoaded,
    ProgressEventTotal,
    ProgressEventLengthComputable,
    IllegalConstructor,
    HeaderIteratorNext,
    HeaderIteratorSelf,
}

pub enum FetchHeaders {
    Own(ApiHeaders),
    Request(Rc<RefCell<FetchObject>>),
    Response(Rc<RefCell<FetchObject>>),
}
pub enum FetchObject {
    Headers(FetchHeaders),
    Request(ApiRequest, AbortSignalId),
    Response(ApiResponse),
    Xhr(Xhr),
    XhrUpload(Rc<RefCell<FetchObject>>),
    ProgressEvent { kind: String, loaded: u64, total: u64, length_computable: bool },
    HeaderIterator { source: Rc<RefCell<FetchObject>>, index: usize, kind: HeaderIterationKind },
}

#[derive(Clone, Copy)]
pub enum HeaderIterationKind { Keys, Values, Entries }

pub trait FetchHost: BindingHost {
    fn fetch_native(
        &mut self,
        name: &'static str,
        length: u8,
        operation: FetchOperation,
        policy: HostCallPolicy,
    ) -> Self::Value;
    fn fetch_register(
        &mut self,
        interface: FetchInterface,
        prototype: Self::Value,
        constructor: Self::Value,
    );
    fn fetch_prototype(&self, interface: FetchInterface) -> Option<Self::Value>;
    fn fetch_allocate(
        &mut self,
        interface: FetchInterface,
        object: FetchObject,
    ) -> Result<Self::Value, Self::Error>;
    fn fetch_record(
        &self,
        value: &Self::Value,
        interface: FetchInterface,
    ) -> Result<Rc<RefCell<FetchObject>>, Self::Error>;
    fn fetch_own_keys(&mut self, value: &Self::Value) -> Result<Vec<String>, Self::Error>;
    fn fetch_base(&self) -> Result<HttpUrl, Self::Error>;
    fn fetch_execute(&mut self, request: &mut ApiRequest) -> Result<ApiResponse, ApiError>;
    fn fetch_start(&mut self, request: ApiRequest, signal: AbortSignalId) -> Result<Self::Value, Self::Error>;
    /// Browser-local file pages retain their existing relative resource loader.
    fn fetch_local_file(&mut self, args: &[Self::Value]) -> Result<Option<Self::Value>, Self::Error>;
    fn fetch_promise(
        &mut self,
        result: Result<Self::Value, ApiError>,
    ) -> Result<Self::Value, Self::Error>;
    fn fetch_promise_value(&mut self, value: Self::Value) -> Result<Self::Value, Self::Error>;
    fn fetch_array_buffer(&mut self, bytes: Vec<u8>) -> Result<Self::Value, Self::Error>;
    fn fetch_uint8_array(&mut self, bytes: Vec<u8>) -> Result<Self::Value, Self::Error>;
    fn fetch_buffer_source_bytes(&mut self, value: &Self::Value) -> Result<Option<Vec<u8>>, Self::Error>;
    fn fetch_array_iterator(&mut self, values: Vec<Self::Value>) -> Result<Self::Value, Self::Error>;
    fn fetch_symbol_iterator(&mut self, prototype: &Self::Value, method: Self::Value) -> Result<(), Self::Error>;
    fn fetch_json_parse(&mut self, text: &str) -> Result<Self::Value, Self::Error>;
    fn fetch_range_error(&mut self, message: &str) -> Self::Error;
    fn fetch_type_error(&mut self, message: &str) -> Self::Error;
    fn fetch_number_value(&self, value: &Self::Value) -> Option<f64>;
    fn fetch_xhr_execute(&mut self, xhr: &mut Xhr) -> Result<Vec<XhrEvent>, XhrError>;
    fn fetch_xhr_start(&mut self, record: Rc<RefCell<FetchObject>>,
        receiver: Self::Value) -> Result<(), Self::Error>;
    fn fetch_upload_has_listeners(&mut self, receiver: &Self::Value) -> Result<bool, Self::Error>;
    fn fetch_xhr_ready_state(&self, receiver: &Self::Value,
        current: amazing_fetch::xhr::XhrState) -> amazing_fetch::xhr::XhrState;
    fn fetch_xhr_event(&mut self, receiver: &Self::Value, event: XhrEvent) -> Result<(), Self::Error>;
    fn fetch_xhr_error(&mut self, error: XhrError) -> Self::Error;
    fn fetch_xhr_document(&mut self, receiver: &Self::Value, xhr: &Xhr) -> Result<Self::Value, Self::Error>;
    fn fetch_reject_throw(&mut self, error: Self::Error) -> Result<Self::Value, Self::Error>;
    fn fetch_add_listener(&mut self, target: &Self::Value, name: String,
        callback: Self::Value) -> Result<(), Self::Error>;
    fn fetch_remove_listener(&mut self, target: &Self::Value, name: &str,
        callback: &Self::Value) -> Result<(), Self::Error>;
    fn fetch_header_iterator(&mut self, source: Rc<RefCell<FetchObject>>,
        kind: HeaderIterationKind) -> Result<Self::Value, Self::Error>;
    fn fetch_iterator_result(&mut self, value: Self::Value, done: bool) -> Result<Self::Value, Self::Error>;
    fn fetch_sequence(&mut self, value: &Self::Value) -> Result<Option<Vec<Vec<Self::Value>>>, Self::Error>;
}

fn member<H: FetchHost>(
    host: &mut H,
    prototype: &H::Value,
    name: &'static str,
    op: FetchOperation,
    length: u8,
) -> Result<(), H::Error> {
    let value = host.fetch_native(name, length, op, HostCallPolicy::CallOnly);
    host.define_data_property(prototype, name, value, true, false, true)
}
fn getter<H: FetchHost>(
    host: &mut H,
    prototype: &H::Value,
    name: &'static str,
    get: FetchOperation,
    set: Option<FetchOperation>,
) -> Result<(), H::Error> {
    let getter = host.fetch_native(name, 0, get, HostCallPolicy::CallOnly);
    let setter = set.map(|op| host.fetch_native(name, 1, op, HostCallPolicy::CallOnly));
    host.define_accessor_property(prototype, name, Some(getter), setter, true, true)
}
fn static_member<H: FetchHost>(
    host: &mut H,
    constructor: &H::Value,
    name: &'static str,
    op: FetchOperation,
    length: u8,
) -> Result<(), H::Error> {
    let value = host.fetch_native(name, length, op, HostCallPolicy::CallOnly);
    host.define_data_property(constructor, name, value, true, false, true)
}

pub fn install<H: FetchHost>(host: &mut H) -> Result<(), H::Error> {
    use FetchInterface as I;
    use FetchOperation as O;
    let global = host.global_object()?;
    for interface in [
        I::Headers,
        I::Request,
        I::Response,
        I::XMLHttpRequest,
        I::XMLHttpRequestUpload,
        I::ProgressEvent,
    ] {
        let prototype = host.create_object(None)?;
        let constructor_op = match interface {
            I::Headers => O::HeadersConstructor,
            I::Request => O::RequestConstructor,
            I::Response => O::ResponseConstructor,
            I::XMLHttpRequest => O::XhrConstructor,
            I::XMLHttpRequestUpload => O::IllegalConstructor,
            I::ProgressEvent => O::ProgressEventConstructor,
            I::HeadersIterator => unreachable!(),
        };
        let length = match interface {
            I::Request => 1,
            I::XMLHttpRequest => 0,
            _ => 0,
        };
        let constructor = host.fetch_native(
            interface.name(),
            length,
            constructor_op,
            HostCallPolicy::ConstructOnly,
        );
        host.define_data_property(
            &constructor,
            "prototype",
            prototype.clone(),
            false,
            false,
            false,
        )?;
        host.define_data_property(
            &prototype,
            "constructor",
            constructor.clone(),
            true,
            false,
            true,
        )?;
        host.fetch_register(interface, prototype.clone(), constructor.clone());
        host.define_data_property(
            &global,
            interface.name(),
            constructor.clone(),
            true,
            false,
            true,
        )?;
        match interface {
            I::Headers => {
                let iterator = host.fetch_native("entries", 0, O::HeadersEntries, HostCallPolicy::CallOnly);
                host.fetch_symbol_iterator(&prototype, iterator)?;
                for (name, op, len) in [
                    ("append", O::HeadersAppend, 2),
                    ("set", O::HeadersSet, 2),
                    ("delete", O::HeadersDelete, 1),
                    ("get", O::HeadersGet, 1),
                    ("has", O::HeadersHas, 1),
                    ("getSetCookie", O::HeadersGetSetCookie, 0),
                    ("forEach", O::HeadersForEach, 1),
                    ("keys", O::HeadersKeys, 0),
                    ("values", O::HeadersValues, 0),
                    ("entries", O::HeadersEntries, 0),
                ] {
                    member(host, &prototype, name, op, len)?;
                }
            }
            I::Request => {
                member(host, &prototype, "clone", O::RequestClone, 0)?;
                for (name, op) in [
                    ("method", O::RequestMethod),
                    ("url", O::RequestUrl),
                    ("headers", O::RequestHeaders),
                    ("mode", O::RequestMode),
                    ("credentials", O::RequestCredentials),
                    ("cache", O::RequestCache),
                    ("redirect", O::RequestRedirect),
                    ("referrer", O::RequestReferrer),
                    ("referrerPolicy", O::RequestReferrerPolicy),
                    ("integrity", O::RequestIntegrity),
                    ("keepalive", O::RequestKeepalive),
                    ("body", O::RequestBody),
                    ("bodyUsed", O::RequestBodyUsed),
                    ("signal", O::RequestSignal),
                ] {
                    getter(host, &prototype, name, op, None)?;
                }
                install_body(host, &prototype)?;
            }
            I::Response => {
                member(host, &prototype, "clone", O::ResponseClone, 0)?;
                static_member(host, &constructor, "error", O::ResponseError, 0)?;
                static_member(host, &constructor, "redirect", O::ResponseRedirect, 1)?;
                for (name, op) in [
                    ("type", O::ResponseType),
                    ("url", O::ResponseUrl),
                    ("redirected", O::ResponseRedirected),
                    ("status", O::ResponseStatus),
                    ("statusText", O::ResponseStatusText),
                    ("ok", O::ResponseOk),
                    ("headers", O::ResponseHeaders),
                    ("body", O::ResponseBody),
                    ("bodyUsed", O::ResponseBodyUsed),
                ] {
                    getter(host, &prototype, name, op, None)?;
                }
                install_body(host, &prototype)?;
            }
            I::XMLHttpRequest => {
                for (name, op, len) in [
                    ("addEventListener", O::XhrAddEventListener, 2),
                    ("removeEventListener", O::XhrRemoveEventListener, 2),
                    ("open", O::XhrOpen, 2),
                    ("setRequestHeader", O::XhrSetRequestHeader, 2),
                    ("send", O::XhrSend, 0),
                    ("abort", O::XhrAbort, 0),
                    ("getResponseHeader", O::XhrGetResponseHeader, 1),
                    ("getAllResponseHeaders", O::XhrGetAllResponseHeaders, 0),
                    ("overrideMimeType", O::XhrOverrideMimeType, 1),
                ] {
                    member(host, &prototype, name, op, len)?;
                }
                for (name, get, set) in [
                    ("readyState", O::XhrReadyState, None),
                    ("status", O::XhrStatus, None),
                    ("statusText", O::XhrStatusText, None),
                    ("responseURL", O::XhrResponseUrl, None),
                    (
                        "responseType",
                        O::XhrResponseType,
                        Some(O::XhrSetResponseType),
                    ),
                    ("responseText", O::XhrResponseText, None),
                    ("response", O::XhrResponse, None),
                    ("responseXML", O::XhrResponseXml, None),
                    ("timeout", O::XhrTimeout, Some(O::XhrSetTimeout)),
                    (
                        "withCredentials",
                        O::XhrWithCredentials,
                        Some(O::XhrSetWithCredentials),
                    ),
                    ("upload", O::XhrUpload, None),
                ] {
                    getter(host, &prototype, name, get, set)?;
                }
                for (name, value) in [
                    ("UNSENT", 0),
                    ("OPENED", 1),
                    ("HEADERS_RECEIVED", 2),
                    ("LOADING", 3),
                    ("DONE", 4),
                ] {
                    let number = host.number(value as f64);
                    host.define_data_property(
                        &constructor,
                        name,
                        number.clone(),
                        false,
                        false,
                        false,
                    )?;
                    host.define_data_property(&prototype, name, number, false, false, false)?;
                }
            }
            I::XMLHttpRequestUpload => {
                for (name, op, len) in [
                    ("addEventListener", O::XhrAddEventListener, 2),
                    ("removeEventListener", O::XhrRemoveEventListener, 2),
                ] { member(host, &prototype, name, op, len)?; }
            }
            I::ProgressEvent => {
                for (name, op) in [("type", O::ProgressEventType),
                    ("loaded", O::ProgressEventLoaded), ("total", O::ProgressEventTotal),
                    ("lengthComputable", O::ProgressEventLengthComputable)] {
                    getter(host, &prototype, name, op, None)?;
                }
            }
            I::HeadersIterator => unreachable!(),
        }
    }
    let fetch = host.fetch_native("fetch", 1, O::Fetch, HostCallPolicy::CallOnly);
    host.define_data_property(&global, "fetch", fetch, true, false, true)
}
fn install_body<H: FetchHost>(host: &mut H, prototype: &H::Value) -> Result<(), H::Error> {
    use FetchOperation as O;
    for (name, op) in [
        ("text", O::BodyText),
        ("json", O::BodyJson),
        ("blob", O::BodyBlob),
        ("arrayBuffer", O::BodyArrayBuffer),
        ("formData", O::BodyFormData),
        ("bytes", O::BodyBytes),
    ] {
        member(host, prototype, name, op, 0)?;
    }
    Ok(())
}

fn required<'a, H: FetchHost>(
    host: &mut H,
    args: &'a [H::Value],
    i: usize,
) -> Result<&'a H::Value, H::Error> {
    args.get(i)
        .ok_or_else(|| host.fetch_type_error("Missing required argument"))
}
fn arg_units<H: FetchHost>(host: &mut H, args: &[H::Value], index: usize) -> Result<Vec<u16>, H::Error> {
    let value = required(host, args, index)?;
    units(host, value)
}
fn arg_usv<H: FetchHost>(host: &mut H, args: &[H::Value], index: usize) -> Result<String, H::Error> {
    let value = required(host, args, index)?;
    usv(host, value)
}
fn units<H: FetchHost>(host: &mut H, value: &H::Value) -> Result<Vec<u16>, H::Error> {
    host.to_dom_string(value)
}
fn usv<H: FetchHost>(host: &mut H, value: &H::Value) -> Result<String, H::Error> {
    Ok(String::from_utf16_lossy(&units(host, value)?))
}
fn string<H: FetchHost>(host: &mut H, value: &str) -> Result<H::Value, H::Error> {
    host.string(&value.encode_utf16().collect::<Vec<_>>())
}
fn error<H: FetchHost>(host: &mut H, value: ApiError) -> H::Error {
    if value.is_range_error() {
        host.fetch_range_error(&format!("{value:?}"))
    } else {
        host.fetch_type_error(&format!("{value:?}"))
    }
}
fn headers_copy<H: FetchHost>(host: &mut H, value: &H::Value) -> Result<ApiHeaders, H::Error> {
    let mut headers = ApiHeaders::default();
    if let Some(rows) = host.fetch_sequence(value)? {
        for row in rows {
            if row.len() != 2 { return Err(host.fetch_type_error("Header tuple must contain two items")); }
            let name = units(host, &row[0])?;
            let value = units(host, &row[1])?;
            headers.append(&name, &value).map_err(|e| error(host, e))?;
        }
        return Ok(headers);
    }
    if let Ok(record) = host.fetch_record(value, FetchInterface::Headers) {
        return Ok(read_headers(&record).map_err(|e| error(host, e))?);
    }
    if host.is_nullish(value) { return Err(host.fetch_type_error("Headers init is null")); }
    let keys = host.fetch_own_keys(value)?;
    let length = host.get_property(value, "length")?;
    if let Some(count) = host.fetch_number_value(&length) {
        if !count.is_finite() || count < 0.0 || count > 100_000.0 {
            return Err(host.fetch_type_error("Invalid Headers sequence length"));
        }
        for index in 0..count as usize {
            let row = host.get_property(value, &index.to_string())?;
            let row_length = host.get_property(&row, "length")?;
            if host.fetch_number_value(&row_length) != Some(2.0) {
                return Err(host.fetch_type_error("Header tuple must contain two items"));
            }
            let name = host.get_property(&row, "0")?;
            let value = host.get_property(&row, "1")?;
            let name = units(host, &name)?;
            let value = units(host, &value)?;
            headers.append(&name, &value).map_err(|e| error(host, e))?;
        }
    } else {
        for name in keys {
            let field = host.get_property(value, &name)?;
            let val = units(host, &field)?;
            headers
                .append(&name.encode_utf16().collect::<Vec<_>>(), &val)
                .map_err(|e| error(host, e))?;
        }
    }
    Ok(headers)
}
fn read_headers(record: &Rc<RefCell<FetchObject>>) -> Result<ApiHeaders, ApiError> {
    match &*record.borrow() {
        FetchObject::Headers(FetchHeaders::Own(headers)) => Ok(headers.clone()),
        FetchObject::Headers(FetchHeaders::Request(parent)) => match &*parent.borrow() {
            FetchObject::Request(request, _) => Ok(request.headers.clone()),
            _ => Err(ApiError::InvalidHeaderName),
        },
        FetchObject::Headers(FetchHeaders::Response(parent)) => match &*parent.borrow() {
            FetchObject::Response(response) => Ok(response.headers.clone()),
            _ => Err(ApiError::InvalidHeaderName),
        },
        _ => Err(ApiError::InvalidHeaderName),
    }
}
fn write_headers(record: &Rc<RefCell<FetchObject>>, headers: ApiHeaders) -> Result<(), ApiError> {
    match &mut *record.borrow_mut() {
        FetchObject::Headers(FetchHeaders::Own(target)) => *target = headers,
        FetchObject::Headers(FetchHeaders::Request(parent)) => match &mut *parent.borrow_mut() {
            FetchObject::Request(request, _) => request.headers = headers,
            _ => return Err(ApiError::InvalidHeaderName),
        },
        FetchObject::Headers(FetchHeaders::Response(parent)) => match &mut *parent.borrow_mut() {
            FetchObject::Response(response) => response.headers = headers,
            _ => return Err(ApiError::InvalidHeaderName),
        },
        _ => return Err(ApiError::InvalidHeaderName),
    }
    Ok(())
}

fn body_input<H: FetchHost>(host: &mut H, value: &H::Value) -> Result<Option<BodyInput>, H::Error> {
    if host.is_nullish(value) {
        return Ok(None);
    }
    if let Some(bytes) = host.fetch_buffer_source_bytes(value)? {
        return Ok(Some(BodyInput::Bytes(bytes)));
    }
    if host.platform_object(value, InterfaceBrand::BLOB).is_some() {
        return host.blob_record(value).map(|v| Some(BodyInput::Blob(v)));
    }
    if host.platform_object(value, InterfaceBrand::FILE).is_some() {
        return host
            .file_record(value)
            .map(|v| Some(BodyInput::Blob(v.blob)));
    }
    if host
        .platform_object(value, InterfaceBrand::FORM_DATA)
        .is_some()
    {
        return host
            .form_data_record(value)
            .map(|v| Some(BodyInput::FormData(v)));
    }
    if host
        .platform_object(value, InterfaceBrand::URL_SEARCH_PARAMS)
        .is_some()
    {
        let params = host.url_params_record(value)?;
        let pairs = params
            .pairs()
            .iter()
            .map(|(a, b)| (a.encode_utf16().collect(), b.encode_utf16().collect()))
            .collect();
        return Ok(Some(BodyInput::UrlSearchParams(pairs)));
    }
    Ok(Some(BodyInput::String(units(host, value)?)))
}
fn xhr_body_input<H: FetchHost>(host: &mut H, value: &H::Value) -> Result<Option<BodyInput>, H::Error> {
    if let Some(node) = host.node_brand(value) {
        let is_document = host.with_dom(|dom| Ok(matches!(&dom.get(node).kind, NodeKind::Document)))?;
        if is_document {
            let content_type = host.get_property(value, "contentType")?;
            let content_type = usv(host, &content_type)?;
            let mime = if content_type.eq_ignore_ascii_case("text/html") {
                "text/html;charset=UTF-8"
            } else {
                "application/xml;charset=UTF-8"
            };
            let markup = host.with_dom(|dom| dom.serialize_children(node))?;
            return Ok(Some(BodyInput::Document {
                bytes: amazing_fetch::api::usv_string(&markup).into_bytes(),
                content_type: mime.into(),
            }));
        }
    }
    body_input(host, value)
}
fn optional<H: FetchHost>(
    host: &mut H,
    object: &H::Value,
    name: &str,
) -> Result<Option<H::Value>, H::Error> {
    let value = host.get_property(object, name)?;
    Ok((!host.is_undefined(&value)).then_some(value))
}
fn request_init<H: FetchHost>(
    host: &mut H,
    value: Option<&H::Value>,
) -> Result<ApiRequestInit, H::Error> {
    let mut init = ApiRequestInit::default();
    let Some(value) = value.filter(|v| !host.is_nullish(v)) else {
        return Ok(init);
    };
    if let Some(v) = optional(host, value, "method")? {
        init.method = Some(units(host, &v)?);
    }
    if let Some(v) = optional(host, value, "headers")? {
        init.headers = Some(headers_copy(host, &v)?);
    }
    if let Some(v) = optional(host, value, "body")? {
        init.body = Some(body_input(host, &v)?);
    }
    if let Some(v) = optional(host, value, "referrer")? {
        init.referrer = Some(units(host, &v)?);
    }
    for (name, target) in [
        ("referrerPolicy", &mut init.referrer_policy),
        ("mode", &mut init.mode),
        ("credentials", &mut init.credentials),
        ("cache", &mut init.cache),
        ("redirect", &mut init.redirect),
        ("integrity", &mut init.integrity),
        ("duplex", &mut init.duplex),
        ("priority", &mut init.priority),
    ] {
        if let Some(v) = optional(host, value, name)? {
            *target = Some(usv(host, &v)?);
        }
    }
    if let Some(v) = optional(host, value, "keepalive")? {
        init.keepalive = Some(host.to_boolean(&v));
    }
    if let Some(v) = optional(host, value, "window")? {
        init.window_non_null = Some(!host.is_nullish(&v));
    }
    Ok(init)
}
fn request<H: FetchHost>(host: &mut H, args: &[H::Value])
    -> Result<(ApiRequest, AbortSignalId), H::Error> {
    let input = required(host, args, 0)?;
    let mut init = request_init(host, args.get(1))?;
    let base = host.fetch_base()?;
    let record = host.fetch_record(input, FetchInterface::Request).ok();
    let inherited = record.as_ref().and_then(|record| match &*record.borrow() {
        FetchObject::Request(_, signal) => Some(*signal), _ => None,
    });
    let explicit = args.get(1).filter(|value| !host.is_nullish(value))
        .map(|value| optional(host, value, "signal")).transpose()?.flatten();
    let source = if let Some(value) = explicit {
        if host.is_nullish(&value) { None } else {
            let key = host.platform_object(&value, InterfaceBrand::ABORT_SIGNAL)
                .ok_or_else(|| host.fetch_type_error("Expected AbortSignal"))?;
            let arena = host.node_platform_key(host.current_document()).arena;
            if key.arena != arena { return Err(host.fetch_type_error("Foreign AbortSignal")); }
            let id = AbortSignalId(key.slot as usize);
            host.with_dom(|dom| dom.abort_signal_checked(id))?;
            Some(id)
        }
    } else { inherited };
    let signal = host.with_dom(|dom| match source {
        Some(source) => dom.create_dependent_abort_signal(&[source]),
        None => Ok(dom.create_abort_signal()),
    })?;
    init.signal_aborted = Some(host.with_dom(|dom| dom.abort_signal_reason(signal))?.is_some());
    let request = if let Some(record) = record {
        let borrow = record.borrow();
        let FetchObject::Request(other, _) = &*borrow else {
            return Err(host.fetch_type_error("Expected Request"));
        };
        ApiRequest::new(ApiRequestInput::Request(other), init, &base, base.origin().clone())
    } else {
        let input = units(host, input)?;
        ApiRequest::new(ApiRequestInput::Url(&input), init, &base, base.origin().clone())
    }.map_err(|e| error(host, e))?;
    Ok((request, signal))
}
fn response_init<H: FetchHost>(
    host: &mut H,
    value: Option<&H::Value>,
) -> Result<ApiResponseInit, H::Error> {
    let mut init = ApiResponseInit::default();
    let Some(value) = value.filter(|v| !host.is_nullish(v)) else {
        return Ok(init);
    };
    if let Some(v) = optional(host, value, "status")? {
        let n = host.to_number(&v)?;
        init.status = Some(if n.is_nan() || n < 0.0 { 0 } else { n as u16 });
    }
    if let Some(v) = optional(host, value, "statusText")? {
        init.status_text = Some(units(host, &v)?);
    }
    if let Some(v) = optional(host, value, "headers")? {
        init.headers = Some(headers_copy(host, &v)?);
    }
    Ok(init)
}
fn wrap_headers<H: FetchHost>(host: &mut H, headers: FetchHeaders) -> Result<H::Value, H::Error> {
    host.fetch_allocate(FetchInterface::Headers, FetchObject::Headers(headers))
}
fn body<H: FetchHost>(
    host: &mut H,
    op: FetchOperation,
    receiver: &H::Value,
) -> Result<H::Value, H::Error> {
    let record = host
        .fetch_record(receiver, FetchInterface::Request)
        .or_else(|_| host.fetch_record(receiver, FetchInterface::Response))?;
    let result = {
        let mut object = record.borrow_mut();
        let body = match &mut *object {
            FetchObject::Request(request, _) => &mut request.body,
            FetchObject::Response(response) => &mut response.body,
            _ => return Err(host.fetch_type_error("Expected Body")),
        };
        match op {
            FetchOperation::BodyText => body.text().map(BodyResult::Text),
            FetchOperation::BodyJson => body.text().map(BodyResult::Json),
            FetchOperation::BodyBlob => body.blob().map(BodyResult::Blob),
            FetchOperation::BodyArrayBuffer | FetchOperation::BodyBytes => {
                body.bytes().map(BodyResult::Bytes)
            }
            FetchOperation::BodyFormData => body.form_data().map(BodyResult::FormData),
            _ => unreachable!(),
        }
    };
    let result = match result {
        Ok(BodyResult::Text(value)) => string(host, &value)?,
        Ok(BodyResult::Json(value)) => match host.fetch_json_parse(&value) {
            Ok(value) => value,
            Err(error) => return host.fetch_reject_throw(error),
        },
        Ok(BodyResult::Blob(value)) => host.blob_new(value)?,
        Ok(BodyResult::Bytes(value)) if op == FetchOperation::BodyBytes =>
            host.fetch_uint8_array(value)?,
        Ok(BodyResult::Bytes(value)) => host.fetch_array_buffer(value)?,
        Ok(BodyResult::FormData(value)) => host.form_data_new(value)?,
        Err(e) => return host.fetch_promise(Err(e)),
    };
    host.fetch_promise_value(result)
}
enum BodyResult {
    Text(String),
    Json(String),
    Blob(amazing_fetch::api::Blob),
    Bytes(Vec<u8>),
    FormData(amazing_fetch::api::FormData),
}

fn json_value<H: FetchHost>(host: &mut H, value: JsonValue) -> Result<H::Value, H::Error> {
    match value {
        JsonValue::Null => Ok(host.null()),
        JsonValue::Boolean(value) => Ok(host.boolean(value)),
        JsonValue::Number(value) => Ok(host.number(value)),
        JsonValue::String(value) => string(host, &value),
        JsonValue::Array(values) => {
            let values = values.into_iter().map(|v| json_value(host, v))
                .collect::<Result<Vec<_>, _>>()?;
            host.create_array(values)
        }
        JsonValue::Object(entries) => {
            let object = host.create_object(None)?;
            for (name, value) in entries {
                let value = json_value(host, value)?;
                host.set_property(&object, &name, value)?;
            }
            Ok(object)
        }
    }
}

pub fn invoke<H: FetchHost>(
    host: &mut H,
    op: FetchOperation,
    receiver: H::Value,
    args: &[H::Value],
) -> Result<H::Value, H::Error> {
    use FetchOperation as O;
    match op {
        O::HeadersConstructor => {
            let headers = if let Some(v) = args.first().filter(|v| !host.is_undefined(v)) {
                headers_copy(host, v)?
            } else {
                ApiHeaders::default()
            };
            wrap_headers(host, FetchHeaders::Own(headers))
        }
        O::HeadersAppend
        | O::HeadersSet
        | O::HeadersDelete
        | O::HeadersGet
        | O::HeadersHas
        | O::HeadersGetSetCookie
        | O::HeadersForEach
        | O::HeadersKeys
        | O::HeadersValues
        | O::HeadersEntries => {
            let record = host.fetch_record(&receiver, FetchInterface::Headers)?;
            let mut headers = read_headers(&record).map_err(|e| error(host, e))?;
            match op {
                O::HeadersAppend | O::HeadersSet => {
                    let name = arg_units(host, args, 0)?;
                    let value = arg_units(host, args, 1)?;
                    if op == O::HeadersAppend {
                        headers.append(&name, &value)
                    } else {
                        headers.set(&name, &value)
                    }
                    .map_err(|e| error(host, e))?;
                    write_headers(&record, headers).map_err(|e| error(host, e))?;
                    Ok(host.undefined())
                }
                O::HeadersDelete => {
                    let name = arg_units(host, args, 0)?;
                    headers.delete(&name).map_err(|e| error(host, e))?;
                    write_headers(&record, headers).map_err(|e| error(host, e))?;
                    Ok(host.undefined())
                }
                O::HeadersGet | O::HeadersHas => {
                    let name = arg_units(host, args, 0)?;
                    if op == O::HeadersHas {
                        let found = headers.has(&name).map_err(|e| error(host, e))?;
                        Ok(host.boolean(found))
                    } else {
                        match headers.get(&name).map_err(|e| error(host, e))? {
                            Some(value) => string(host, &value),
                            None => Ok(host.null()),
                        }
                    }
                }
                O::HeadersGetSetCookie => {
                    let values = headers.get_set_cookie().iter().map(|s| string(host, s))
                        .collect::<Result<Vec<_>, _>>()?;
                    host.create_array(values)
                },
                O::HeadersForEach => {
                    let callback = required(host, args, 0)?.clone();
                    if !host.is_callable(&callback) {
                        return Err(host.fetch_type_error("Callback is not callable"));
                    }
                    let this = args.get(1).cloned().unwrap_or_else(|| host.undefined());
                    let mut index = 0;
                    loop {
                        let current = read_headers(&record).map_err(|e| error(host, e))?.sorted_entries();
                        let Some((name, value)) = current.get(index).cloned() else { break; };
                        let value = string(host, &value)?;
                        let name = string(host, &name)?;
                        host.call(&callback, &this, &[value, name, receiver.clone()])?;
                        index += 1;
                    }
                    Ok(host.undefined())
                }
                O::HeadersKeys | O::HeadersValues | O::HeadersEntries => {
                    let kind = match op { O::HeadersKeys => HeaderIterationKind::Keys,
                        O::HeadersValues => HeaderIterationKind::Values,
                        _ => HeaderIterationKind::Entries };
                    host.fetch_header_iterator(record.clone(), kind)
                }
                _ => unreachable!(),
            }
        }
        O::RequestConstructor => {
            let (value, signal) = request(host, args)?;
            host.fetch_allocate(FetchInterface::Request, FetchObject::Request(value, signal))
        }
        O::Fetch => {
            if let Some(promise) = host.fetch_local_file(args)? {
                return Ok(promise);
            }
            let (request, signal) = match request(host, args) {
                Ok(request) => request, Err(error) => return host.fetch_reject_throw(error),
            };
            host.fetch_start(request, signal)
        }
        O::RequestClone
        | O::RequestMethod
        | O::RequestUrl
        | O::RequestHeaders
        | O::RequestMode
        | O::RequestCredentials
        | O::RequestCache
        | O::RequestRedirect
        | O::RequestReferrer
        | O::RequestReferrerPolicy
        | O::RequestIntegrity
        | O::RequestKeepalive
        | O::RequestBody
        | O::RequestBodyUsed
        | O::RequestSignal => {
            let record = host.fetch_record(&receiver, FetchInterface::Request)?;
            let object = record.borrow();
            let FetchObject::Request(request, signal) = &*object else {
                return Err(host.fetch_type_error("Expected Request"));
            };
            match op {
                O::RequestClone => {
                    let value = request.clone_request().map_err(|e| error(host, e))?;
                    let signal = host.with_dom(|dom| dom.create_dependent_abort_signal(&[*signal]))?;
                    host.fetch_allocate(FetchInterface::Request, FetchObject::Request(value, signal))
                }
                O::RequestMethod => string(host, request.method()),
                O::RequestUrl => string(host, &request.url().serialize()),
                O::RequestHeaders => wrap_headers(host, FetchHeaders::Request(record.clone())),
                O::RequestMode => string(
                    host,
                    match request.mode {
                        amazing_fetch::RequestMode::Cors => "cors",
                        amazing_fetch::RequestMode::NoCors => "no-cors",
                        amazing_fetch::RequestMode::SameOrigin => "same-origin",
                        amazing_fetch::RequestMode::Navigate => "navigate",
                    },
                ),
                O::RequestCredentials => string(
                    host,
                    match request.credentials {
                        amazing_fetch::CredentialsMode::Omit => "omit",
                        amazing_fetch::CredentialsMode::SameOrigin => "same-origin",
                        amazing_fetch::CredentialsMode::Include => "include",
                    },
                ),
                O::RequestCache => string(
                    host,
                    match request.cache {
                        amazing_fetch::api::ApiCache::Default => "default",
                        amazing_fetch::api::ApiCache::NoStore => "no-store",
                        amazing_fetch::api::ApiCache::Reload => "reload",
                        amazing_fetch::api::ApiCache::NoCache => "no-cache",
                        amazing_fetch::api::ApiCache::ForceCache => "force-cache",
                        amazing_fetch::api::ApiCache::OnlyIfCached => "only-if-cached",
                    },
                ),
                O::RequestRedirect => string(
                    host,
                    match request.redirect {
                        amazing_fetch::RedirectMode::Follow => "follow",
                        amazing_fetch::RedirectMode::Error => "error",
                        amazing_fetch::RedirectMode::Manual => "manual",
                    },
                ),
                O::RequestReferrer => string(host, &request.referrer),
                O::RequestReferrerPolicy => string(
                    host,
                    match request.referrer_policy {
                        amazing_fetch::ReferrerPolicy::NoReferrer => "no-referrer",
                        amazing_fetch::ReferrerPolicy::NoReferrerWhenDowngrade => "no-referrer-when-downgrade",
                        amazing_fetch::ReferrerPolicy::SameOrigin => "same-origin",
                        amazing_fetch::ReferrerPolicy::Origin => "origin",
                        amazing_fetch::ReferrerPolicy::StrictOrigin => "strict-origin",
                        amazing_fetch::ReferrerPolicy::OriginWhenCrossOrigin => "origin-when-cross-origin",
                        amazing_fetch::ReferrerPolicy::StrictOriginWhenCrossOrigin => "strict-origin-when-cross-origin",
                        amazing_fetch::ReferrerPolicy::UnsafeUrl => "unsafe-url",
                    },
                ),
                O::RequestIntegrity => string(host, &request.integrity),
                O::RequestKeepalive => Ok(host.boolean(request.keepalive)),
                O::RequestBody => Ok(host.null()),
                O::RequestBodyUsed => Ok(host.boolean(request.body_used())),
                O::RequestSignal => {
                    let key = PlatformKey { arena: host.node_platform_key(host.current_document()).arena,
                        slot: signal.0 as u64, generation: 1 };
                    host.wrap_platform_object(key, InterfaceBrand::ABORT_SIGNAL)
                }
                _ => unreachable!(),
            }
        }
        O::ResponseConstructor => {
            let body = args
                .first()
                .map(|v| body_input(host, v))
                .transpose()?
                .flatten();
            let init = response_init(host, args.get(1))?;
            let value = ApiResponse::new(body, init).map_err(|e| error(host, e))?;
            host.fetch_allocate(FetchInterface::Response, FetchObject::Response(value))
        }
        O::ResponseError => host.fetch_allocate(
            FetchInterface::Response,
            FetchObject::Response(ApiResponse::error()),
        ),
        O::ResponseRedirect => {
            let target = arg_units(host, args, 0)?;
            let status = if let Some(value) = args.get(1) {
                host.to_number(value)? as u16
            } else {
                302
            };
            let base = host.fetch_base()?;
            let response =
                ApiResponse::redirect(&target, status, &base).map_err(|e| error(host, e))?;
            host.fetch_allocate(FetchInterface::Response, FetchObject::Response(response))
        }
        O::ResponseClone
        | O::ResponseType
        | O::ResponseUrl
        | O::ResponseRedirected
        | O::ResponseStatus
        | O::ResponseStatusText
        | O::ResponseOk
        | O::ResponseHeaders
        | O::ResponseBody
        | O::ResponseBodyUsed => {
            let record = host.fetch_record(&receiver, FetchInterface::Response)?;
            let object = record.borrow();
            let FetchObject::Response(response) = &*object else {
                return Err(host.fetch_type_error("Expected Response"));
            };
            match op {
                O::ResponseClone => {
                    let value = response.clone_response().map_err(|e| error(host, e))?;
                    host.fetch_allocate(FetchInterface::Response, FetchObject::Response(value))
                }
                O::ResponseType => string(
                    host,
                    match response.response_type {
                        amazing_fetch::api::ApiResponseType::Default => "default",
                        amazing_fetch::api::ApiResponseType::Basic => "basic",
                        amazing_fetch::api::ApiResponseType::Cors => "cors",
                        amazing_fetch::api::ApiResponseType::Opaque => "opaque",
                        amazing_fetch::api::ApiResponseType::OpaqueRedirect => "opaqueredirect",
                        amazing_fetch::api::ApiResponseType::Error => "error",
                    },
                ),
                O::ResponseUrl => string(host, &response.url),
                O::ResponseRedirected => Ok(host.boolean(response.redirected)),
                O::ResponseStatus => Ok(host.number(response.status as f64)),
                O::ResponseStatusText => string(host, &response.status_text),
                O::ResponseOk => Ok(host.boolean(response.ok())),
                O::ResponseHeaders => wrap_headers(host, FetchHeaders::Response(record.clone())),
                O::ResponseBody => Ok(host.null()),
                O::ResponseBodyUsed => Ok(host.boolean(response.body_used())),
                _ => unreachable!(),
            }
        }
        O::BodyText
        | O::BodyJson
        | O::BodyBlob
        | O::BodyArrayBuffer
        | O::BodyFormData
        | O::BodyBytes => body(host, op, &receiver),
        O::XhrConstructor => host.fetch_allocate(FetchInterface::XMLHttpRequest,
            FetchObject::Xhr(Xhr::default())),
        O::XhrAddEventListener | O::XhrRemoveEventListener => {
            let _ = host.fetch_record(&receiver, FetchInterface::XMLHttpRequest)
                .or_else(|_| host.fetch_record(&receiver, FetchInterface::XMLHttpRequestUpload))?;
            let name = arg_usv(host, args, 0)?;
            let callback = required(host, args, 1)?.clone();
            if op == O::XhrAddEventListener { host.fetch_add_listener(&receiver, name, callback)?; }
            else if op == O::XhrRemoveEventListener { host.fetch_remove_listener(&receiver, &name, &callback)?; }
            Ok(host.undefined())
        }
        O::ProgressEventConstructor => {
            let kind = arg_usv(host, args, 0)?;
            let mut loaded = 0; let mut total = 0; let mut length_computable = false;
            if let Some(init) = args.get(1).filter(|v| !host.is_nullish(v)) {
                if let Some(v) = optional(host, init, "loaded")? { loaded = host.to_number(&v)? as u64; }
                if let Some(v) = optional(host, init, "total")? { total = host.to_number(&v)? as u64; }
                if let Some(v) = optional(host, init, "lengthComputable")? { length_computable = host.to_boolean(&v); }
            }
            host.fetch_allocate(FetchInterface::ProgressEvent, FetchObject::ProgressEvent {
                kind, loaded, total, length_computable })
        }
        O::ProgressEventType | O::ProgressEventLoaded | O::ProgressEventTotal | O::ProgressEventLengthComputable => {
            let record = host.fetch_record(&receiver, FetchInterface::ProgressEvent)?;
            let record = record.borrow();
            let FetchObject::ProgressEvent { kind, loaded, total, length_computable } = &*record
                else { return Err(host.fetch_type_error("Expected ProgressEvent")); };
            match op {
                O::ProgressEventType => string(host, kind),
                O::ProgressEventLoaded => Ok(host.number(*loaded as f64)),
                O::ProgressEventTotal => Ok(host.number(*total as f64)),
                _ => Ok(host.boolean(*length_computable)),
            }
        }
        O::IllegalConstructor => Err(host.fetch_type_error("Illegal constructor")),
        O::HeaderIteratorSelf => {
            let _ = host.fetch_record(&receiver, FetchInterface::HeadersIterator)?;
            Ok(receiver)
        }
        O::HeaderIteratorNext => {
            let record = host.fetch_record(&receiver, FetchInterface::HeadersIterator)?;
            let (entry, kind) = {
                let mut iterator = record.borrow_mut();
                let FetchObject::HeaderIterator { source, index, kind } = &mut *iterator
                    else { return Err(host.fetch_type_error("Invalid Headers iterator")); };
                let entries = read_headers(source).map_err(|e| error(host, e))?.sorted_entries();
                let entry = entries.get(*index).cloned();
                *index += usize::from(entry.is_some());
                (entry, *kind)
            };
            let Some((name, value)) = entry else {
                return host.fetch_iterator_result(host.undefined(), true);
            };
            let value = match kind {
                HeaderIterationKind::Keys => string(host, &name)?,
                HeaderIterationKind::Values => string(host, &value)?,
                HeaderIterationKind::Entries => {
                    let name = string(host, &name)?;
                    let value = string(host, &value)?;
                    host.create_array(vec![name, value])?
                }
            };
            host.fetch_iterator_result(value, false)
        }
        O::XhrOpen | O::XhrSetRequestHeader | O::XhrSend | O::XhrAbort |
        O::XhrGetResponseHeader | O::XhrGetAllResponseHeaders | O::XhrOverrideMimeType |
        O::XhrReadyState | O::XhrStatus | O::XhrStatusText | O::XhrResponseUrl |
        O::XhrResponseType | O::XhrSetResponseType | O::XhrResponseText | O::XhrResponse |
        O::XhrResponseXml |
        O::XhrTimeout | O::XhrSetTimeout | O::XhrWithCredentials | O::XhrSetWithCredentials |
        O::XhrUpload => xhr_invoke(host, op, &receiver, args),
    }
}


struct Utf8Decoder;
impl XhrTextDecoder for Utf8Decoder {
    fn decode(&self, bytes: &[u8], _charset: Option<&str>) -> String {
        String::from_utf8_lossy(bytes).into_owned()
    }
}
fn xhr_invoke<H: FetchHost>(host: &mut H, op: FetchOperation,
    receiver: &H::Value, args: &[H::Value]) -> Result<H::Value, H::Error> {
    use FetchOperation as O;
    let record = host.fetch_record(receiver, FetchInterface::XMLHttpRequest)?;
    match op {
        O::XhrOpen => {
            let method = arg_units(host, args, 0)?;
            let url = arg_units(host, args, 1)?;
            let async_flag = args.get(2).map(|v| host.to_boolean(v)).unwrap_or(true);
            let base = host.fetch_base()?;
            let events = {
                let mut record = record.borrow_mut();
                let FetchObject::Xhr(xhr) = &mut *record else { return Err(host.fetch_type_error("Expected XMLHttpRequest")); };
                xhr.open(&method, &url, async_flag, &base, base.origin().clone())
                    .map_err(|e| host.fetch_xhr_error(e))?
            };
            for event in events { host.fetch_xhr_event(receiver, event)?; }
            Ok(host.undefined())
        }
        O::XhrSetRequestHeader => {
            let name = arg_units(host, args, 0)?;
            let value = arg_units(host, args, 1)?;
            let mut record = record.borrow_mut();
            let FetchObject::Xhr(xhr) = &mut *record else { return Err(host.fetch_type_error("Expected XMLHttpRequest")); };
            xhr.set_request_header(&name, &value).map_err(|e| host.fetch_xhr_error(e))?;
            Ok(host.undefined())
        }
        O::XhrSend => {
            let body = args.first().map(|v| xhr_body_input(host, v)).transpose()?.flatten();
            let upload_has_listeners = host.fetch_upload_has_listeners(receiver)?;
            let events = {
                let mut record = record.borrow_mut();
                let FetchObject::Xhr(xhr) = &mut *record else { return Err(host.fetch_type_error("Expected XMLHttpRequest")); };
                xhr.set_upload_listeners(upload_has_listeners);
                xhr.send(body, 0).map_err(|e| host.fetch_xhr_error(e))?
            };
            for event in events { host.fetch_xhr_event(receiver, event)?; }
            let asynchronous = {
                let record = record.borrow();
                let FetchObject::Xhr(xhr) = &*record else { unreachable!() };
                if !xhr.send_invoked() { return Ok(host.undefined()); }
                xhr.is_asynchronous()
            };
            if asynchronous {
                host.fetch_xhr_start(record.clone(), receiver.clone())?;
            } else {
                let events = {
                    let mut record = record.borrow_mut();
                    let FetchObject::Xhr(xhr) = &mut *record else { unreachable!() };
                    host.fetch_xhr_execute(xhr).map_err(|e| host.fetch_xhr_error(e))?
                };
                for event in events { host.fetch_xhr_event(receiver, event)?; }
            }
            Ok(host.undefined())
        }
        O::XhrAbort => {
            let events = { let mut record = record.borrow_mut();
                let FetchObject::Xhr(xhr) = &mut *record else { return Err(host.fetch_type_error("Expected XMLHttpRequest")); };
                xhr.abort() };
            for event in events { host.fetch_xhr_event(receiver, event)?; }
            Ok(host.undefined())
        }
        O::XhrGetResponseHeader => {
            let name = arg_units(host, args, 0)?;
            let record = record.borrow();
            let FetchObject::Xhr(xhr) = &*record else { return Err(host.fetch_type_error("Expected XMLHttpRequest")); };
            match xhr.get_response_header(&name).map_err(|e| host.fetch_xhr_error(e))? {
                Some(value) => string(host, &value), None => Ok(host.null()),
            }
        }
        O::XhrGetAllResponseHeaders => {
            let record = record.borrow();
            let FetchObject::Xhr(xhr) = &*record else { return Err(host.fetch_type_error("Expected XMLHttpRequest")); };
            string(host, &xhr.get_all_response_headers())
        }
        O::XhrOverrideMimeType => {
            let mime = arg_usv(host, args, 0)?;
            let mut record = record.borrow_mut();
            let FetchObject::Xhr(xhr) = &mut *record else { return Err(host.fetch_type_error("Expected XMLHttpRequest")); };
            xhr.override_mime_type(&mime).map_err(|e| host.fetch_xhr_error(e))?;
            Ok(host.undefined())
        }
        O::XhrSetResponseType | O::XhrSetTimeout | O::XhrSetWithCredentials => {
            let value = required(host, args, 0)?;
            let kind = if op == O::XhrSetResponseType { Some(usv(host, value)?) } else { None };
            let millis = if op == O::XhrSetTimeout { Some(host.to_number(value)? as u32) } else { None };
            let credentials = if op == O::XhrSetWithCredentials { Some(host.to_boolean(value)) } else { None };
            let mut record = record.borrow_mut();
            let FetchObject::Xhr(xhr) = &mut *record else { return Err(host.fetch_type_error("Expected XMLHttpRequest")); };
            match op {
                O::XhrSetResponseType => {
                    let kind = match kind.as_deref().unwrap() {
                        "" => XhrResponseType::Default, "text" => XhrResponseType::Text,
                        "json" => XhrResponseType::Json, "arraybuffer" => XhrResponseType::ArrayBuffer,
                        "document" => XhrResponseType::Document, "blob" => XhrResponseType::Blob,
                        _ => return Ok(host.undefined()),
                    };
                    xhr.set_response_type(kind)
                }
                O::XhrSetTimeout => xhr.set_timeout(millis.unwrap()),
                _ => xhr.set_with_credentials(credentials.unwrap()),
            }.map_err(|e| host.fetch_xhr_error(e))?;
            Ok(host.undefined())
        }
        O::XhrUpload => host.fetch_allocate(FetchInterface::XMLHttpRequestUpload,
            FetchObject::XhrUpload(record.clone())),
        _ => {
            let record = record.borrow();
            let FetchObject::Xhr(xhr) = &*record else { return Err(host.fetch_type_error("Expected XMLHttpRequest")); };
            match op {
                O::XhrReadyState => Ok(host.number(host.fetch_xhr_ready_state(receiver, xhr.state()) as u8 as f64)),
                O::XhrStatus => Ok(host.number(xhr.status() as f64)),
                O::XhrStatusText => string(host, xhr.status_text()),
                O::XhrResponseUrl => string(host, xhr.response_url()),
                O::XhrResponseType => string(host, match xhr.response_type() {
                    XhrResponseType::Default => "", XhrResponseType::Text => "text",
                    XhrResponseType::Json => "json", XhrResponseType::ArrayBuffer => "arraybuffer",
                    XhrResponseType::Document => "document", XhrResponseType::Blob => "blob" }),
                O::XhrResponseText => {
                    let value = xhr.response_text(&Utf8Decoder).map_err(|e| host.fetch_xhr_error(e))?;
                    string(host, &value)
                },
                O::XhrResponseXml => {
                    if !matches!(xhr.response_type(), XhrResponseType::Default | XhrResponseType::Document) {
                        return Err(host.fetch_xhr_error(XhrError::InvalidStateResponseXml));
                    }
                    host.fetch_xhr_document(receiver, xhr)
                }
                O::XhrResponse if xhr.response_type() == XhrResponseType::Document =>
                    host.fetch_xhr_document(receiver, xhr),
                O::XhrResponse => match xhr.response(&Utf8Decoder).map_err(|e| host.fetch_xhr_error(e))? {
                    XhrResponseValue::Text(value) => string(host, &value),
                    XhrResponseValue::ArrayBuffer(bytes) => host.fetch_array_buffer(bytes),
                    XhrResponseValue::Blob(blob) => host.blob_new(blob),
                    XhrResponseValue::Json(Some(value)) => json_value(host, value),
                    XhrResponseValue::Json(None) | XhrResponseValue::Null => Ok(host.null()),
                },
                O::XhrTimeout => Ok(host.number(xhr.timeout() as f64)),
                O::XhrWithCredentials => Ok(host.boolean(xhr.with_credentials())),
                _ => Err(host.fetch_type_error("Unsupported XHR operation")),
            }
        }
    }
}
