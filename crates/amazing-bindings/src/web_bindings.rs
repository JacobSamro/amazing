//! WebIDL bindings for URL and body models. Coercion happens before native mutation.
use crate::host::{BindingHost, InterfaceBrand as B, Operation as O};
use amazing_dom::{DomError, NodeIdx};
use amazing_fetch::api::{Blob, BlobPart, File, FormData, FormValue};
use amazing_net::{url::{UrlSearchParams, UrlWithSearchParams}, ParsedUrl};

pub fn handles(op: O) -> bool {
    matches!(op,
        O::URLConstructor | O::URLCanParse | O::URLParse | O::URLHref | O::URLSetHref |
        O::URLOrigin | O::URLProtocol | O::URLSetProtocol | O::URLUsername | O::URLSetUsername |
        O::URLPassword | O::URLSetPassword | O::URLHost | O::URLSetHost | O::URLHostname |
        O::URLSetHostname | O::URLPort | O::URLSetPort | O::URLPathname | O::URLSetPathname |
        O::URLSearch | O::URLSetSearch | O::URLSearchParams | O::URLHash | O::URLSetHash |
        O::URLToString | O::URLToJSON | O::URLCreateObjectURL | O::URLRevokeObjectURL | O::URLSearchParamsConstructor |
        O::URLSearchParamsAppend | O::URLSearchParamsDelete | O::URLSearchParamsGet |
        O::URLSearchParamsGetAll | O::URLSearchParamsHas | O::URLSearchParamsSet |
        O::URLSearchParamsSort | O::URLSearchParamsSize | O::URLSearchParamsToString |
        O::URLSearchParamsForEach | O::BlobConstructor | O::BlobSize | O::BlobType |
        O::BlobSlice | O::BlobText | O::BlobArrayBuffer | O::BlobBytes | O::BlobStream |
        O::FileConstructor | O::FileName | O::FileLastModified | O::FormDataConstructor |
        O::FormDataAppend | O::FormDataDelete | O::FormDataGet | O::FormDataGetAll |
        O::FormDataHas | O::FormDataSet | O::FormDataForEach | O::DocumentFonts |
        O::FontFaceSetStatus | O::FontFaceSetReady)
}
fn required<'a, H: BindingHost>(host: &mut H, args: &'a [H::Value], index: usize) -> Result<&'a H::Value, H::Error> {
    args.get(index).ok_or_else(|| host.type_error("Missing required argument"))
}
fn usv<H: BindingHost>(host: &mut H, value: &H::Value) -> Result<String, H::Error> {
    Ok(String::from_utf16_lossy(&host.to_dom_string(value)?))
}
fn required_usv<H: BindingHost>(host: &mut H, args: &[H::Value], index: usize) -> Result<String, H::Error> {
    let value = required(host, args, index)?;
    usv(host, value)
}
fn string<H: BindingHost>(host: &mut H, value: &str) -> Result<H::Value, H::Error> {
    host.string(&value.encode_utf16().collect::<Vec<_>>())
}
fn check<H: BindingHost>(host: &mut H, receiver: &H::Value, brand: B) -> Result<(), H::Error> {
    if host.platform_object(receiver, brand).is_some() { Ok(()) }
    else { Err(host.type_error("Incompatible receiver")) }
}
fn parsed<H: BindingHost>(host: &mut H, args: &[H::Value]) -> Result<Result<ParsedUrl, ()>, H::Error> {
    let input = required_usv(host, args, 0)?;
    let base = if let Some(value) = args.get(1) {
        if host.is_undefined(value) { None } else { Some(usv(host, value)?) }
    } else { None };
    let base = match base {
        Some(base) => match ParsedUrl::parse(&base) { Ok(url) => Some(url), Err(_) => return Ok(Err(())) },
        None => None,
    };
    Ok(ParsedUrl::parse_with_base(&input, base.as_ref()).map_err(|_| ()))
}
fn url<H: BindingHost>(host: &mut H, op: O, receiver: H::Value, args: &[H::Value]) -> Result<H::Value, H::Error> {
    if matches!(op, O::URLConstructor | O::URLCanParse | O::URLParse) {
        let parsed = parsed(host, args)?;
        return match op {
            O::URLCanParse => Ok(host.boolean(parsed.is_ok())),
            O::URLParse => match parsed { Ok(url) => {
                let model = UrlWithSearchParams::parse(&url.serialize()).map_err(|_| host.type_error("Invalid URL"))?;
                host.url_new(model)
            }, Err(()) => Ok(host.null()) },
            _ => match parsed { Ok(url) => {
                let model = UrlWithSearchParams::parse(&url.serialize()).map_err(|_| host.type_error("Invalid URL"))?;
                host.url_new(model)
            }, Err(()) => Err(host.type_error("Invalid URL")) },
        };
    }
    if op == O::URLCreateObjectURL {
        let value = required(host, args, 0)?.clone();
        let blob = if host.platform_object(&value, B::FILE).is_some() || host.platform_object(&value, B::BLOB).is_some() {
            blob_value(host, &value)?
        } else { return Err(host.type_error("URL.createObjectURL requires a Blob")); };
        let origin = host.blob_url_origin()?;
        let created = amazing_fetch::blob_url::create_object_url(&origin, blob);
        return string(host, &created);
    }
    if op == O::URLRevokeObjectURL {
        let input = required_usv(host, args, 0)?;
        amazing_fetch::blob_url::revoke_object_url(&input);
        return Ok(host.undefined());
    }
    check(host, &receiver, B::URL)?;
    if op == O::URLSearchParams { return host.url_search_params(&receiver); }
    let model = host.url_record(&receiver)?;
    let record = model.url();
    let value = match op {
        O::URLHref | O::URLToString | O::URLToJSON => record.serialize(),
        O::URLOrigin => record.origin().serialize(),
        O::URLProtocol => format!("{}:", record.scheme()),
        O::URLUsername => record.username().into(), O::URLPassword => record.password().into(),
        O::URLHost => record.host_and_port(), O::URLHostname => record.hostname(),
        O::URLPort => record.port().map(|n| n.to_string()).unwrap_or_default(),
        O::URLPathname => record.pathname(),
        O::URLSearch => record.query().map(|q| format!("?{q}")).unwrap_or_default(),
        O::URLHash => record.fragment().map(|f| format!("#{f}")).unwrap_or_default(),
        _ => {
            let input = required_usv(host, args, 0)?;
            if op == O::URLSetHref {
                model.set_href(&input).map_err(|_| host.type_error("Invalid URL"))?;
            } else {
                let mut updated = record;
                match op {
                    O::URLSetProtocol => updated.set_protocol(&input),
                    O::URLSetUsername => updated.set_username(&input),
                    O::URLSetPassword => updated.set_password(&input),
                    O::URLSetHost => updated.set_host(&input),
                    O::URLSetHostname => updated.set_hostname(&input),
                    O::URLSetPort => updated.set_port(&input),
                    O::URLSetPathname => updated.set_pathname(&input),
                    O::URLSetSearch => updated.set_search(&input),
                    O::URLSetHash => updated.set_hash(&input),
                    _ => return Err(host.type_error("Unknown URL operation")),
                }
                // The linked URLSearchParams record shares the same URL through
                // UrlWithSearchParams, so all setters update it in place.
                model.set_href(&updated.serialize()).map_err(|_| host.type_error("Invalid URL"))?;
            }
            return Ok(host.undefined());
        }
    };
    string(host, &value)
}
fn params<H: BindingHost>(host: &mut H, op: O, receiver: H::Value, args: &[H::Value]) -> Result<H::Value, H::Error> {
    if op == O::URLSearchParamsConstructor {
        let model = if let Some(init) = args.first() {
            if host.is_undefined(init) { UrlSearchParams::new("") }
            else if let Some(units) = host.string_units(init) { UrlSearchParams::new(&String::from_utf16_lossy(&units)) }
            else if let Some(pairs) = host.url_params_pairs(init)? { UrlSearchParams::from_pairs(pairs) }
            else { UrlSearchParams::new(&usv(host, init)?) }
        } else { UrlSearchParams::new("") };
        return host.url_params_new(model);
    }
    check(host, &receiver, B::URL_SEARCH_PARAMS)?;
    let mut model = host.url_params_record(&receiver)?;
    let name = if matches!(op, O::URLSearchParamsAppend | O::URLSearchParamsDelete |
        O::URLSearchParamsGet | O::URLSearchParamsGetAll | O::URLSearchParamsHas | O::URLSearchParamsSet) {
        Some(required_usv(host, args, 0)?)
    } else { None };
    let second = if matches!(op, O::URLSearchParamsAppend | O::URLSearchParamsSet) {
        Some(required_usv(host, args, 1)?)
    } else if matches!(op, O::URLSearchParamsDelete | O::URLSearchParamsHas) {
        args.get(1).filter(|v| !host.is_undefined(v)).map(|v| usv(host, v)).transpose()?
    } else { None };
    match op {
        O::URLSearchParamsAppend => model.append(name.as_deref().unwrap(), second.as_deref().unwrap()),
        O::URLSearchParamsDelete => model.delete(name.as_deref().unwrap(), second.as_deref()),
        O::URLSearchParamsSet => model.set(name.as_deref().unwrap(), second.as_deref().unwrap()),
        O::URLSearchParamsSort => model.sort(),
        O::URLSearchParamsGet => return model.get(name.as_deref().unwrap()).map(|s| string(host, &s)).unwrap_or_else(|| Ok(host.null())),
        O::URLSearchParamsGetAll => {
            let values = model.get_all(name.as_deref().unwrap()).iter().map(|s| string(host, s)).collect::<Result<Vec<_>,_>>()?;
            return host.create_array(values);
        }
        O::URLSearchParamsHas => return Ok(host.boolean(model.has(name.as_deref().unwrap(), second.as_deref()))),
        O::URLSearchParamsSize => return Ok(host.number(model.size() as f64)),
        O::URLSearchParamsToString => return string(host, &model.to_string()),
        O::URLSearchParamsForEach => {
            let callback = required(host, args, 0)?.clone();
            if !host.is_callable(&callback) { return Err(host.type_error("Callback is not callable")); }
            let this = args.get(1).cloned().unwrap_or_else(|| host.undefined());
            let mut index = 0;
            loop {
                let pairs = host.url_params_record(&receiver)?.pairs();
                let Some((name, value)) = pairs.get(index) else { break };
                let name = string(host, name)?;
                let value = string(host, value)?;
                host.call(&callback, &this, &[value, name, receiver.clone()])?;
                index += 1;
            }
            return Ok(host.undefined());
        }
        _ => return Err(host.type_error("Unknown URLSearchParams operation")),
    }
    host.url_params_store(&receiver, model)?;
    Ok(host.undefined())
}
fn fonts<H: BindingHost>(host: &mut H, op: O, receiver: H::Value) -> Result<H::Value, H::Error> {
    match op {
        O::DocumentFonts => {
            let document = host.node_brand(&receiver).ok_or_else(|| host.type_error("Expected Document"))?;
            host.with_dom(|dom| if matches!(dom.get(document).kind, amazing_dom::NodeKind::Document) { Ok(()) } else { Err(DomError::TypeError) })?;
            host.font_face_set(document)
        }
        O::FontFaceSetStatus => {
            check(host, &receiver, B::FONT_FACE_SET)?;
            let status = if host.font_face_set_pending(&receiver)? { "loading" } else { "loaded" };
            string(host, status)
        }
        O::FontFaceSetReady => {
            check(host, &receiver, B::FONT_FACE_SET)?;
            host.font_face_set_ready(&receiver)
        }
        _ => Err(host.type_error("Unknown FontFaceSet operation")),
    }
}

fn blob_value<H: BindingHost>(host: &mut H, receiver: &H::Value) -> Result<Blob, H::Error> {
    if host.platform_object(receiver, B::FILE).is_some() {
        Ok(host.file_record(receiver)?.blob)
    } else {
        check(host, receiver, B::BLOB)?;
        host.blob_record(receiver)
    }
}
/// WebIDL BlobPropertyBag / FilePropertyBag. Members convert in lexicographic
/// order: `endings`, `lastModified` (File only), `type`.
struct BlobBag { kind: String, native_endings: bool, last_modified: Option<i64> }
fn blob_bag<H: BindingHost>(host: &mut H, value: Option<&H::Value>, file: bool) -> Result<BlobBag, H::Error> {
    let mut bag = BlobBag { kind: String::new(), native_endings: false, last_modified: None };
    let Some(value) = value else { return Ok(bag) };
    if host.is_nullish(value) { return Ok(bag) }
    host.require_object(value)?;
    let endings = host.get_property(value, "endings")?;
    if !host.is_undefined(&endings) {
        match usv(host, &endings)?.as_str() {
            "transparent" => {}
            "native" => bag.native_endings = true,
            _ => return Err(host.type_error("Invalid endings value")),
        }
    }
    if file {
        let modified = host.get_property(value, "lastModified")?;
        if !host.is_undefined(&modified) {
            let n = host.to_number(&modified)?;
            bag.last_modified = Some(if n.is_finite() { n.trunc() as i64 } else { 0 });
        }
    }
    let kind = host.get_property(value, "type")?;
    if !host.is_undefined(&kind) { bag.kind = usv(host, &kind)?; }
    Ok(bag)
}
/// File API "convert line endings to native": the native newline is LF.
fn native_endings(parts: Vec<BlobPart>) -> Vec<BlobPart> {
    parts.into_iter().map(|part| match part {
        BlobPart::Text(text) => BlobPart::Text(text.replace("\r\n", "\n").replace('\r', "\n")),
        other => other,
    }).collect()
}
/// WebIDL `[Clamp] long long`: ToNumber, clamp, round half to even.
fn clamped<H: BindingHost>(host: &mut H, value: Option<&H::Value>) -> Result<Option<i64>, H::Error> {
    let Some(value) = value else { return Ok(None) };
    if host.is_undefined(value) { return Ok(None) }
    let n = host.to_number(value)?;
    Ok(Some(if n.is_nan() { 0 } else { n.round_ties_even() as i64 }))
}
fn blob<H: BindingHost>(host: &mut H, op: O, receiver: H::Value, args: &[H::Value]) -> Result<H::Value, H::Error> {
    if op == O::BlobConstructor {
        let parts = match args.first() { Some(v) if !host.is_undefined(v) => host.blob_parts(v)?, _ => Vec::new() };
        let bag = blob_bag(host, args.get(1), false)?;
        let parts = if bag.native_endings { native_endings(parts) } else { parts };
        return host.blob_new(Blob::new(parts, &bag.kind));
    }
    if op == O::FileConstructor {
        let input = required(host, args, 0)?;
        let parts = host.blob_parts(input)?;
        let name = required_usv(host, args, 1)?.replace('/', ":");
        let bag = blob_bag(host, args.get(2), true)?;
        let parts = if bag.native_endings { native_endings(parts) } else { parts };
        let last_modified = bag.last_modified.unwrap_or_else(now_ms);
        return host.file_new(File::new(parts, &name, &bag.kind, last_modified));
    }
    if op == O::FileName || op == O::FileLastModified {
        check(host, &receiver, B::FILE)?;
        let file = host.file_record(&receiver)?;
        return if op == O::FileName { string(host, &file.name) }
            else { Ok(host.number(file.last_modified as f64)) };
    }
    let model = blob_value(host, &receiver)?;
    match op {
        O::BlobSize => Ok(host.number(model.size() as f64)),
        O::BlobType => string(host, &model.content_type),
        O::BlobSlice => {
            let start = clamped(host, args.first())?;
            let end = clamped(host, args.get(1))?;
            let kind = match args.get(2) { Some(v) if !host.is_undefined(v) => usv(host, v)?, _ => String::new() };
            host.blob_new(model.slice(start, end, &kind))
        }
        O::BlobText => host.blob_promise_bytes(model.bytes, true, false),
        O::BlobArrayBuffer => host.blob_promise_bytes(model.bytes, false, true),
        O::BlobBytes => host.blob_promise_bytes(model.bytes, false, false),
        O::BlobStream => host.blob_stream(model.bytes),
        _ => Err(host.type_error("Unknown Blob operation")),
    }
}
fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default().as_millis() as i64
}
fn submitted_form(host_dom: &amazing_dom::Dom, form: NodeIdx) -> Result<FormData, DomError> {
    let mut result = FormData::default();
    for control in host_dom.form_controls(form)? {
        if host_dom.attribute_utf16(control, "disabled").is_some() { continue }
        let Some(name) = host_dom.attribute_utf16(control, "name") else { continue };
        if name.is_empty() { continue }
        let name = String::from_utf16_lossy(&name);
        if host_dom.html_tag(control, "input") {
            let kind = host_dom.input_type(control)?;
            if matches!(kind, "button" | "reset" | "submit" | "image") { continue }
            if matches!(kind, "checkbox" | "radio") && !host_dom.input_checked(control)? { continue }
            if kind == "file" {
                result.append_file(&name, File::new([], "", "application/octet-stream", 0), None);
            } else {
                result.append_text(&name, &String::from_utf16_lossy(&host_dom.input_value(control)?));
            }
        } else if host_dom.html_tag(control, "textarea") {
            result.append_text(&name, &String::from_utf16_lossy(&host_dom.textarea_value(control)?));
        } else if host_dom.html_tag(control, "select") {
            for option in host_dom.select_options(control)? {
                if host_dom.option_selected(option)? {
                    result.append_text(&name, &String::from_utf16_lossy(&host_dom.option_value(option)?));
                }
            }
        }
    }
    Ok(result)
}
fn form_value<H: BindingHost>(host: &mut H, value: &H::Value, filename: Option<&H::Value>) -> Result<FormValue, H::Error> {
    if host.platform_object(value, B::FILE).is_some() {
        let mut file = host.file_record(value)?;
        if let Some(filename) = filename { file.name = usv(host, filename)?; }
        return Ok(FormValue::File(file));
    }
    if host.platform_object(value, B::BLOB).is_some() {
        let blob = host.blob_record(value)?;
        let name = filename.map(|v| usv(host, v)).transpose()?.unwrap_or_else(|| "blob".into());
        return Ok(FormValue::File(File { blob, name, last_modified: now_ms() }));
    }
    if filename.is_some() { return Err(host.type_error("Filename requires a Blob")); }
    Ok(FormValue::Text(usv(host, value)?))
}
fn form_value_js<H: BindingHost>(host: &mut H, value: &FormValue) -> Result<H::Value, H::Error> {
    match value {
        FormValue::Text(s) => string(host, s),
        FormValue::File(f) => host.file_new(f.clone()),
    }
}
fn form_data<H: BindingHost>(host: &mut H, op: O, receiver: H::Value, args: &[H::Value]) -> Result<H::Value, H::Error> {
    if op == O::FormDataConstructor {
        let data = if let Some(form) = args.first().filter(|v| !host.is_undefined(v)) {
            let node = host.node_brand(form).ok_or_else(|| host.type_error("Expected HTMLFormElement"))?;
            host.with_dom(|dom| submitted_form(dom, node))?
        } else { FormData::default() };
        return host.form_data_new(data);
    }
    check(host, &receiver, B::FORM_DATA)?;
    let mut data = host.form_data_record(&receiver)?;
    if op == O::FormDataForEach {
        let callback = required(host, args, 0)?.clone();
        if !host.is_callable(&callback) { return Err(host.type_error("Callback is not callable")); }
        let this = args.get(1).cloned().unwrap_or_else(|| host.undefined());
        let mut index = 0;
        loop {
            let current = host.form_data_record(&receiver)?;
            let Some((name, value)) = current.entries.get(index) else { break };
            let name = string(host, name)?;
            let value = form_value_js(host, value)?;
            host.call(&callback, &this, &[value, name, receiver.clone()])?;
            index += 1;
        }
        return Ok(host.undefined());
    }
    let name = required_usv(host, args, 0)?;
    match op {
        O::FormDataAppend | O::FormDataSet => {
            let value = required(host, args, 1)?.clone();
            let entry = form_value(host, &value, args.get(2))?;
            if op == O::FormDataAppend { data.entries.push((name, entry)); }
            else { data.set(&name, entry); }
        }
        O::FormDataDelete => data.delete(&name),
        O::FormDataGet => return data.get(&name).map(|v| form_value_js(host, v)).unwrap_or_else(|| Ok(host.null())),
        O::FormDataGetAll => {
            let values = data.get_all(&name).iter().map(|v| form_value_js(host, v)).collect::<Result<Vec<_>,_>>()?;
            return host.create_array(values);
        }
        O::FormDataHas => return Ok(host.boolean(data.has(&name))),
        _ => return Err(host.type_error("Unknown FormData operation")),
    }
    host.form_data_store(&receiver, data)?;
    Ok(host.undefined())
}
pub fn invoke<H: BindingHost>(host: &mut H, op: O, receiver: H::Value, args: &[H::Value]) -> Result<H::Value, H::Error> {
    if matches!(op, O::DocumentFonts | O::FontFaceSetStatus | O::FontFaceSetReady) { return fonts(host, op, receiver); }
    if matches!(op, O::URLConstructor | O::URLCanParse | O::URLParse | O::URLHref | O::URLSetHref |
        O::URLOrigin | O::URLProtocol | O::URLSetProtocol | O::URLUsername | O::URLSetUsername |
        O::URLPassword | O::URLSetPassword | O::URLHost | O::URLSetHost | O::URLHostname |
        O::URLSetHostname | O::URLPort | O::URLSetPort | O::URLPathname | O::URLSetPathname |
        O::URLSearch | O::URLSetSearch | O::URLSearchParams | O::URLHash | O::URLSetHash |
        O::URLToString | O::URLToJSON | O::URLCreateObjectURL | O::URLRevokeObjectURL) { return url(host, op, receiver, args); }
    if matches!(op, O::URLSearchParamsConstructor | O::URLSearchParamsAppend |
        O::URLSearchParamsDelete | O::URLSearchParamsGet | O::URLSearchParamsGetAll |
        O::URLSearchParamsHas | O::URLSearchParamsSet | O::URLSearchParamsSort |
        O::URLSearchParamsSize | O::URLSearchParamsToString | O::URLSearchParamsForEach) {
        return params(host, op, receiver, args);
    }
    if matches!(op, O::BlobConstructor | O::BlobSize | O::BlobType | O::BlobSlice | O::BlobText |
        O::BlobArrayBuffer | O::BlobBytes | O::BlobStream | O::FileConstructor |
        O::FileName | O::FileLastModified) { return blob(host, op, receiver, args); }
    if matches!(op, O::FormDataConstructor | O::FormDataAppend | O::FormDataDelete |
        O::FormDataGet | O::FormDataGetAll | O::FormDataHas | O::FormDataSet |
        O::FormDataForEach) { return form_data(host, op, receiver, args); }
    Err(host.type_error("Unknown web binding"))
}
