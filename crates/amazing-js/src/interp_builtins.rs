// Portions ported from boa-dev/boa (core/engine/src/builtins/error/{mod,aggregate,type}.rs)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT.
// Error-family allocation, metadata and iterable algorithms adapted to Amazing.
// Portions ported from boa-dev/boa (core/engine/src/builtins/array/mod.rs)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT.
// Generic Array search/reduction length protocols; other builtin algorithms unchanged.
// Portions ported from boa-dev/boa (core/engine/src/builtins/promise/mod.rs)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT:
// JSRT-30 optional result-capability completion in Promise reaction jobs;
// JSRT-53 complete intrinsic Promise constructor allocation/order algorithm.
// Portions ported from boa-dev/boa (core/engine/src/builtins/object/mod.rs)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT: prototype tags, own-property order and static primitive boundaries.
// Portions ported from boa-dev/boa (core/engine/src/builtins/array/mod.rs)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT: Array.isArray semantic dispatch.

fn native_object(i: &mut Interpreter, this: JsValue, a: Vec<JsValue>) -> Result<JsValue, Throw> {
    if let (Some(new_target), Some(active)) = (i.current_new_target(), i.native_stack.last()) {
        let is_object_constructor = active.borrow().func.as_ref().is_some_and(|function|
            matches!(&function.body, FunctionBody::Native(native)
                if std::ptr::fn_addr_eq(*native, native_object as crate::value::NativeFn)));
        if is_object_constructor && new_target != JsValue::Object(active.clone()) { return Ok(this); }
    }
    Ok(match a.into_iter().next() {
        Some(value @ JsValue::Object(_)) => value,
        Some(JsValue::Num(number)) => {
            let mut object = JsObject::ordinary();
object.proto = Some(i.number_proto.clone());
            object.boxed_primitive = Some(JsValue::Num(number));
            JsValue::object(object)
        }
        Some(JsValue::Bool(boolean)) => {
            let mut object = JsObject::ordinary();
object.proto = Some(i.boolean_proto.clone());
            object.boxed_primitive = Some(JsValue::Bool(boolean));
            JsValue::object(object)
        }
        Some(JsValue::Str(string)) => {
            let mut object = JsObject::ordinary();
            object.proto = Some(i.string_proto.clone());
            object.initialize_string_data(string);
            JsValue::object(object)
        }
        Some(JsValue::BigInt(value)) => {
            let mut object = JsObject::ordinary();
object.proto = Some(i.bigint_proto.clone());
            object.boxed_primitive = Some(JsValue::BigInt(value));
            JsValue::object(object)
        }
        Some(JsValue::Symbol(symbol)) => {
            let mut object = JsObject::ordinary();
            object.proto = Some(i.symbol_proto.clone());
            object.exotic = Exotic::SymbolObject(symbol);
            JsValue::object(object)
        }
        _ => i.object_value(),
    })
}

fn dom_id(this: &JsValue) -> Result<u32, Throw> {
    let JsValue::Object(object) = this else {
        return Err(Throw::type_error("DOM method called on non-element"));
    };
    match dom_node_id(object) {
        Some(id) => Ok(id),
        None => Err(Throw::type_error("DOM method called on non-element")),
    }
}

fn dom_node_id(object: &Obj) -> Option<u32> {
    match object.borrow().get_own("[[DomNode]]") {
        Some(JsValue::Num(n)) if n.is_finite() && n >= 0.0 && n.fract() == 0.0 => Some(n as u32),
        _ => None,
    }
}

fn dom_host(i: &mut Interpreter) -> Result<Rc<RefCell<dyn DomHost>>, Throw> {
    i.dom_host
        .as_ref()
        .cloned()
        .ok_or_else(|| Throw::type_error("DOM host is not attached"))
}

fn dom_mutation<T>(
    i: &mut Interpreter,
    operation: impl FnOnce(&mut dyn DomHost) -> Result<T, String>,
) -> Result<T, Throw> {
    let host = dom_host(i)?;
    let result = {
        let mut borrowed = host.borrow_mut();
        operation(&mut *borrowed)
    };
    // Release the arena borrow before appending native work to the JS outbox.
    i.flush_native_microtasks()?;
    result.map_err(Throw::type_error)
}

fn native_dom_query_selector(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let selector = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    let id = dom_host(i)?.borrow_mut().query_selector(&selector);
    Ok(id.map(|id| i.make_dom_element(id)).unwrap_or(JsValue::Null))
}

fn native_dom_query_selector_all(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let selector = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    let ids = dom_host(i)?.borrow_mut().query_selector_all(&selector);
    let values = ids.into_iter().map(|id| i.make_dom_element(id)).collect();
    array_values(i, values)
}

fn native_dom_get_element_by_id(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let id = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    let selector = format!("#{id}");
    let node = dom_host(i)?.borrow_mut().query_selector(&selector);
    Ok(node
        .map(|node| i.make_dom_element(node))
        .unwrap_or(JsValue::Null))
}

fn native_dom_get_elements_by_tag(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let tag = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    let ids = dom_host(i)?.borrow_mut().query_selector_all(&tag);
    array_values(
        i,
        ids.into_iter().map(|id| i.make_dom_element(id)).collect(),
    )
}

fn native_dom_get_elements_by_class(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let classes = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    let selector = classes
        .split_ascii_whitespace()
        .filter(|class| !class.is_empty())
        .map(|class| format!(".{class}"))
        .collect::<String>();
    let ids = dom_host(i)?.borrow_mut().query_selector_all(&selector);
    array_values(
        i,
        ids.into_iter().map(|id| i.make_dom_element(id)).collect(),
    )
}

fn native_dom_create_element(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let name = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    let id = dom_mutation(i, |host| host.create_element(&name))?;
    Ok(i.make_dom_element(id))
}

fn native_dom_create_text_node(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let text = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    let id = dom_mutation(i, |host| host.create_text_node(&text))?;
    Ok(i.make_dom_element(id))
}

fn native_dom_text_get(
    i: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let id = dom_id(&this)?;
    Ok(dom_host(i)?
        .borrow()
        .get_text_content(id)
        .map(JsValue::str)
        .unwrap_or(JsValue::Undefined))
}

fn native_dom_text_set(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let id = dom_id(&this)?;
    let text = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    dom_mutation(i, |host| host.set_text_content(id, &text))?;
    Ok(JsValue::Undefined)
}

fn native_dom_inner_html_get(
    i: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let node = dom_id(&this)?;
    Ok(dom_host(i)?
        .borrow()
        .get_inner_html(node)
        .map(JsValue::str)
        .unwrap_or(JsValue::Undefined))
}

fn native_dom_inner_html_set(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let node = dom_id(&this)?;
    let html = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    dom_mutation(i, |host| host.set_inner_html(node, &html))?;
    Ok(JsValue::Undefined)
}

fn native_dom_title_get(
    i: &mut Interpreter,
    _: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    Ok(JsValue::str(dom_host(i)?.borrow().get_document_title()))
}

fn native_dom_title_set(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let title = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    dom_mutation(i, |host| host.set_document_title(&title))?;
    Ok(JsValue::Undefined)
}

fn native_location_href_get(
    i: &mut Interpreter,
    _: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    Ok(JsValue::str(i.location_url.clone()))
}

fn native_location_href_set(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let url = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    i.location_url = url.clone();
    i.pending_navigation = Some(url);
    Ok(JsValue::Undefined)
}

fn native_location_assign(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    native_location_href_set(i, JsValue::Undefined, args)
}

fn native_location_reload(
    i: &mut Interpreter,
    _: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    i.pending_navigation = Some(i.location_url.clone());
    Ok(JsValue::Undefined)
}

fn native_window_location_get(
    i: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let JsValue::Object(window) = this else {
        return Err(Throw::type_error(
            "location getter receiver is not an object",
        ));
    };
    let window = if let Some(target) = i.window_proxy_target(
        &window, WindowProxyOperation::Get, Some(&PropertyKey::from("location")),
    ) { target?.1 } else { window };
    Ok(window
        .borrow()
        .get_own("[[LocationObject]]")
        .unwrap_or(JsValue::Undefined))
}

fn native_window_location_set(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    native_location_href_set(i, JsValue::Undefined, args)
}

fn native_history_back(i: &mut Interpreter, _: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    i.pending_history_delta = Some(-1);
    Ok(JsValue::Undefined)
}

fn native_history_forward(
    i: &mut Interpreter,
    _: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    i.pending_history_delta = Some(1);
    Ok(JsValue::Undefined)
}

fn native_history_go(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let delta = i.to_number(args.first().unwrap_or(&JsValue::num(0.0)))?;
    if delta.is_finite() {
        i.pending_history_delta =
            Some(delta.trunc().clamp(i32::MIN as f64, i32::MAX as f64) as i32);
    }
    Ok(JsValue::Undefined)
}

fn native_history_push_state(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let state = args.first().cloned().unwrap_or(JsValue::Null);
    let url = args
        .get(2)
        .map(|value| i.to_string(value))
        .transpose()?
        .unwrap_or_else(|| i.location_url.clone());
    i.location_url = url.clone();
    i.set_history_state(state.clone());
    i.pending_history_update = Some((url, false, state));
    Ok(JsValue::Undefined)
}

fn native_history_replace_state(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let state = args.first().cloned().unwrap_or(JsValue::Null);
    let url = args
        .get(2)
        .map(|value| i.to_string(value))
        .transpose()?
        .unwrap_or_else(|| i.location_url.clone());
    i.location_url = url.clone();
    i.set_history_state(state.clone());
    i.pending_history_update = Some((url, true, state));
    Ok(JsValue::Undefined)
}

fn native_storage_get_item(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let key = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    let origin = i.location_url.clone();
    Ok(dom_host(i)?
        .borrow()
        .storage_get(&origin, &key)
        .map_or(JsValue::Null, JsValue::str))
}

fn native_storage_set_item(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let key = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    let value = i.to_string(args.get(1).unwrap_or(&JsValue::Undefined))?;
    let origin = i.location_url.clone();
    dom_mutation(i, |host| host.storage_set(&origin, &key, &value))?;
    Ok(JsValue::Undefined)
}

fn native_storage_remove_item(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let key = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    let origin = i.location_url.clone();
    dom_mutation(i, |host| host.storage_remove(&origin, &key))?;
    Ok(JsValue::Undefined)
}

fn native_storage_clear(
    i: &mut Interpreter,
    _: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let origin = i.location_url.clone();
    dom_mutation(i, |host| host.storage_clear(&origin))?;
    Ok(JsValue::Undefined)
}

fn native_fetch(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let input = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    let base = i.location_url.clone();
    let result = dom_host(i)?.borrow_mut().fetch(&base, &input);
    match result {
        Ok(response) => {
            let value = fetch_response(i, response);
            Ok(new_promise(i, PromiseFate::Fulfilled(value)))
        }
        Err(error) => Ok(new_promise(i, PromiseFate::Rejected(JsValue::str(error)))),
    }
}

fn fetch_response(i: &mut Interpreter, response: HostResponse) -> JsValue {
    let object = i.object_value();
    if let JsValue::Object(object) = &object {
        object
            .borrow_mut()
            .set_own("ok", JsValue::Bool((200..300).contains(&response.status)));
        object
            .borrow_mut()
            .set_own("status", JsValue::num(response.status as f64));
        object
            .borrow_mut()
            .set_own("url", JsValue::str(response.url));
        object
            .borrow_mut()
            .set_own("[[Body]]", JsValue::str(response.body));
        let mut headers = JsObject::ordinary();
        headers.proto = Some(i.object_proto.clone());
        for (name, value) in response.headers {
            headers.set_own(&name.to_ascii_lowercase(), JsValue::str(value));
        }
        headers.set_own(
            "get",
            Interpreter::native("get", native_response_headers_get, &i.function_proto),
        );
        headers.set_own(
            "has",
            Interpreter::native("has", native_response_headers_has, &i.function_proto),
        );
        object
            .borrow_mut()
            .set_own("headers", JsValue::object(headers));
        object.borrow_mut().set_own(
            "text",
            Interpreter::native("text", native_response_text, &i.function_proto),
        );
        object.borrow_mut().set_own(
            "json",
            Interpreter::native("json", native_response_json, &i.function_proto),
        );
    }
    object
}

fn native_response_text(
    i: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let JsValue::Object(object) = this else {
        return Err(Throw::type_error("Response.text receiver is not an object"));
    };
    let body = object
        .borrow()
        .get_own("[[Body]]")
        .unwrap_or(JsValue::str(""));
    Ok(new_promise(i, PromiseFate::Fulfilled(body)))
}

fn native_response_json(
    i: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let JsValue::Object(object) = this else {
        return Err(Throw::type_error("Response.json receiver is not an object"));
    };
    let body = object
        .borrow()
        .get_own("[[Body]]")
        .unwrap_or(JsValue::str(""));
    let value = native_json_parse(i, JsValue::Undefined, vec![body])?;
    Ok(new_promise(i, PromiseFate::Fulfilled(value)))
}

fn native_response_headers_get(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let JsValue::Object(headers) = this else {
        return Err(Throw::type_error("Headers.get receiver is not an object"));
    };
    let name = i
        .to_string(args.first().unwrap_or(&JsValue::Undefined))?
        .to_ascii_lowercase();
    Ok(headers.borrow().get_own(&name).unwrap_or(JsValue::Null))
}

fn native_response_headers_has(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let JsValue::Object(headers) = this else {
        return Err(Throw::type_error("Headers.has receiver is not an object"));
    };
    let name = i
        .to_string(args.first().unwrap_or(&JsValue::Undefined))?
        .to_ascii_lowercase();
    Ok(JsValue::Bool(headers.borrow().get_own(&name).is_some()))
}

fn native_dom_get_attribute(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let id = dom_id(&this)?;
    let name = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    match dom_host(i)?.borrow().get_attribute(id, &name) {
        Some(Some(value)) => Ok(JsValue::str(value)),
        Some(None) => Ok(JsValue::Null),
        None => Ok(JsValue::Undefined),
    }
}

fn native_dom_set_attribute(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let id = dom_id(&this)?;
    let name = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    let value = i.to_string(args.get(1).unwrap_or(&JsValue::Undefined))?;
    dom_mutation(i, |host| host.set_attribute(id, &name, &value))?;
    Ok(JsValue::Undefined)
}

fn native_dom_has_attribute(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let node = dom_id(&this)?;
    let name = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    Ok(JsValue::Bool(
        dom_host(i)?
            .borrow()
            .get_attribute(node, &name)
            .is_some_and(|value| value.is_some()),
    ))
}

fn native_dom_remove_attribute(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let node = dom_id(&this)?;
    let name = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    dom_mutation(i, |host| host.remove_attribute(node, &name))?;
    Ok(JsValue::Undefined)
}

fn native_dom_append(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let parent = dom_id(&this)?;
    for arg in args {
        match arg {
            JsValue::Object(_) => {
                let child = dom_id(&arg)?;
                dom_mutation(i, |host| host.append_child(parent, child))?;
            }
            other => {
                let text = i.to_string(&other)?;
                dom_mutation(i, |host| host.append_text(parent, &text))?;
            }
        }
    }
    Ok(JsValue::Undefined)
}

fn native_dom_append_child(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let child = args
        .first()
        .cloned()
        .ok_or_else(|| Throw::type_error("appendChild requires a node"))?;
    native_dom_append(i, this, vec![child.clone()])?;
    Ok(child)
}

fn native_dom_remove(
    i: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let node = dom_id(&this)?;
    dom_mutation(i, |host| host.remove_node(node))?;
    Ok(JsValue::Undefined)
}

fn native_dom_matches(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let node = dom_id(&this)?;
    let selector = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    Ok(JsValue::Bool(
        dom_host(i)?.borrow().matches_selector(node, &selector),
    ))
}

fn native_dom_closest(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let scope = dom_id(&this)?;
    let mut node = scope;
    let selector = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    loop {
        let matches = dom_host(i)?.borrow().matches_selector_scoped(node, &selector, scope);
        if matches {
            return Ok(i.make_dom_element(node));
        }
        let parent = dom_host(i)?.borrow().parent_node(node);
        let Some(parent) = parent else { break };
        node = parent;
    }
    Ok(JsValue::Null)
}

fn class_tokens(i: &mut Interpreter, node: u32) -> Result<Vec<String>, Throw> {
    let value = dom_host(i)?
        .borrow()
        .get_attribute(node, "class")
        .flatten()
        .unwrap_or_default();
    Ok(value.split_ascii_whitespace().map(str::to_owned).collect())
}

fn write_class_tokens(i: &mut Interpreter, node: u32, tokens: &[String]) -> Result<(), Throw> {
    dom_mutation(i, |host| host.set_attribute(node, "class", &tokens.join(" ")))
}

fn class_list_node(this: &JsValue) -> Result<u32, Throw> {
    let JsValue::Object(object) = this else {
        return Err(Throw::type_error(
            "classList method receiver is not an object",
        ));
    };
    Interpreter::dom_class_list_id(object)
        .ok_or_else(|| Throw::type_error("classList method receiver is not a DOM element"))
}

fn native_class_list_contains(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let node = class_list_node(&this)?;
    let token = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    Ok(JsValue::Bool(
        class_tokens(i, node)?.iter().any(|item| item == &token),
    ))
}

fn native_class_list_add(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let node = class_list_node(&this)?;
    let mut tokens = class_tokens(i, node)?;
    for arg in args {
        let token = i.to_string(&arg)?;
        if !token.is_empty() && !tokens.iter().any(|item| item == &token) {
            tokens.push(token);
        }
    }
    write_class_tokens(i, node, &tokens)?;
    Ok(JsValue::Undefined)
}

fn native_class_list_remove(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let node = class_list_node(&this)?;
    let removals: Vec<String> = args
        .iter()
        .map(|arg| i.to_string(arg))
        .collect::<Result<_, _>>()?;
    let mut tokens = class_tokens(i, node)?;
    tokens.retain(|item| !removals.iter().any(|remove| remove == item));
    write_class_tokens(i, node, &tokens)?;
    Ok(JsValue::Undefined)
}

fn native_class_list_toggle(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let node = class_list_node(&this)?;
    let token = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    let force = args.get(1).map(JsValue::to_boolean);
    let mut tokens = class_tokens(i, node)?;
    let present = tokens.iter().any(|item| item == &token);
    let enabled = force.unwrap_or(!present);
    if enabled && !present {
        tokens.push(token.clone());
    }
    if !enabled {
        tokens.retain(|item| item != &token);
    }
    write_class_tokens(i, node, &tokens)?;
    Ok(JsValue::Bool(enabled))
}

fn native_class_list_to_string(
    i: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let node = class_list_node(&this)?;
    Ok(JsValue::str(class_tokens(i, node)?.join(" ")))
}

fn native_dom_add_event_listener(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let this = if this.is_null() || this.is_undefined() {
        JsValue::Object(i.global_obj.clone())
    } else { this };
    let node = dom_id(&this)?;
    let event = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    let callback = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    if callable_function(&callback).is_none() {
        return Err(Throw::type_error("event listener must be callable"));
    }
    i.register_dom_listener(node, event, callback);
    Ok(JsValue::Undefined)
}

fn native_dom_remove_event_listener(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let this = if this.is_null() || this.is_undefined() {
        JsValue::Object(i.global_obj.clone())
    } else { this };
    let node = dom_id(&this)?;
    let event = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    let callback = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    i.dom_listeners.retain(|listener| {
        listener.property
            || listener.node != node
            || listener.event != event
            || listener.callback != callback
    });
    Ok(JsValue::Undefined)
}

fn native_dom_click(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let node = dom_id(&this)?;
    i.dispatch_dom_event(node, "click")?;
    Ok(JsValue::Undefined)
}

fn native_dom_onclick_get(
    i: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let node = dom_id(&this)?;
    Ok(i.dom_handler(node, "click").unwrap_or(JsValue::Null))
}

fn native_dom_onclick_set(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let node = dom_id(&this)?;
    let callback = args.first().cloned().unwrap_or(JsValue::Null);
    if !callback.is_null() && !callback.is_undefined() && callable_function(&callback).is_none() {
        return Err(Throw::type_error("onclick must be callable or null"));
    }
    let callback = if callback.is_null() || callback.is_undefined() {
        None
    } else {
        Some(callback)
    };
    i.set_dom_handler(node, "click", callback);
    Ok(JsValue::Undefined)
}

fn native_event_stop_propagation(
    _: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let JsValue::Object(event) = this else {
        return Err(Throw::type_error(
            "stopPropagation receiver is not an event",
        ));
    };
    event
        .borrow_mut()
        .set_own("cancelBubble", JsValue::Bool(true));
    Ok(JsValue::Undefined)
}

fn native_event_prevent_default(
    _: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let JsValue::Object(event) = this else {
        return Err(Throw::type_error("preventDefault receiver is not an event"));
    };
    event
        .borrow_mut()
        .set_own("defaultPrevented", JsValue::Bool(true));
    Ok(JsValue::Undefined)
}

fn native_object_create(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let prototype = args.first().cloned().unwrap_or(JsValue::Undefined);
    let object = match prototype {
        JsValue::Null => JsObject::ordinary(),
        JsValue::Object(prototype) => {
            let mut object = JsObject::ordinary();
            object.proto = Some(prototype);
            object
        }
        _ => {
            return Err(Throw::type_error(
                "Object prototype must be an object or null",
            ))
        }
    };
    let result = JsValue::object(object);
    if let Some(properties) = args.get(1) {
        if !properties.is_undefined() {
            native_object_define_properties(
                i,
                JsValue::Undefined,
                vec![result.clone(), properties.clone()],
            )?;
        }
    }
    Ok(result)
}

fn native_object_get_prototype_of(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let target = args.first().cloned().unwrap_or(JsValue::Undefined);
    if target.is_null() || target.is_undefined() { return Err(Throw::type_error("getPrototypeOf target is nullish")); }
    let JsValue::Object(object) = native_object(i, JsValue::Undefined, vec![target])? else { unreachable!() };
    i.default_get_prototype(&object)
}

fn native_object_set_prototype_of(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let target = args.first().cloned().unwrap_or(JsValue::Undefined);
    if target.is_null() || target.is_undefined() {
        return Err(Throw::type_error("setPrototypeOf target is nullish"));
    }
    if !matches!(args.get(1), Some(JsValue::Object(_) | JsValue::Null)) {
        return Err(Throw::type_error("prototype must be an object or null"));
    }
    if !target.is_object() { return Ok(target); }
    if std_reflect_set_prototype_of(i, JsValue::Undefined, args)? != JsValue::Bool(true) {
        return Err(Throw::type_error("cannot change object prototype"));
    }
    Ok(target)
}

fn native_object_has_own(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let object = match args.first() {
        Some(JsValue::Object(object)) => object,
        Some(JsValue::Symbol(_)) => return Ok(JsValue::Bool(false)),
        _ => return Err(Throw::type_error("hasOwn target is not an object")),
    };
    let key = i.to_property_key(args.get(1).unwrap_or(&JsValue::Undefined))?;
    Ok(JsValue::Bool(i.default_get_own_property(object, &key)?.is_some()))
}

fn native_array_is_array(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    Ok(JsValue::Bool(is_array_value(i, args.first().unwrap_or(&JsValue::Undefined))?))
}

fn native_number_is_nan(
    _: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    Ok(JsValue::Bool(
        matches!(args.first(), Some(JsValue::Num(value)) if value.is_nan()),
    ))
}

fn native_number_is_finite(
    _: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    Ok(JsValue::Bool(
        matches!(args.first(), Some(JsValue::Num(value)) if value.is_finite()),
    ))
}

fn native_function_call(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    if callable_function(&this).is_none() {
        return Err(Throw::type_error(
            "Function.prototype.call receiver is not callable",
        ));
    }
    let receiver = args.first().cloned().unwrap_or(JsValue::Undefined);
    i.call_value(&this, receiver, args.into_iter().skip(1).collect())
}

fn native_function_apply(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    if callable_function(&this).is_none() {
        return Err(Throw::type_error(
            "Function.prototype.apply receiver is not callable",
        ));
    }
    let receiver = args.first().cloned().unwrap_or(JsValue::Undefined);
    let arguments = match args.get(1) {
        None | Some(JsValue::Undefined) | Some(JsValue::Null) => Vec::new(),
        Some(value) => i.create_list_from_array_like(value)?,
    };
    i.call_value(&this, receiver, arguments)
}

fn native_function_bind(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    if callable_function(&this).is_none() {
        return Err(Throw::type_error(
            "Function.prototype.bind receiver is not callable",
        ));
    }
    let bound_count = args.len().saturating_sub(1);
    let target_has_length = matches!(&this, JsValue::Object(object) if object.borrow().own_prop("length").is_some());
    let target_length = if target_has_length { i.get_prop(&this, "length")? } else { JsValue::Undefined };
    let target_length = match target_length {
        JsValue::Num(number) if number.is_finite() => number.trunc().max(0.0),
        JsValue::Num(number) if number.is_infinite() && number.is_sign_positive() => f64::INFINITY,
        _ => 0.0,
    };
    let target_name = i.get_prop(&this, "name")?;
    let target_name = match target_name { JsValue::Str(name) => name.to_utf8_lossy(), _ => String::new() };
    let target_constructible = matches!(&this, JsValue::Object(object) if object.borrow().constructible);
    let data = Obj(Rc::new(RefCell::new(JsObject::ordinary())));
    data.borrow_mut().set_own("target", this.clone());
    data.borrow_mut()
        .set_own("this", args.first().cloned().unwrap_or(JsValue::Undefined));
    data.borrow_mut()
        .set_own("args", array_values(i, args.into_iter().skip(1).collect())?);
    let bound = Interpreter::native("bound", native_bound, &i.function_proto);
    if let JsValue::Object(function) = &bound {
        let mut function = function.borrow_mut();
        if let JsValue::Object(target) = &this {
            function.proto = target.borrow().proto.clone();
            function.realm = target.borrow().realm;
        }
        function.func.as_mut().unwrap().home = Some(data);
        function.constructible = target_constructible;
        function.delete_own("prototype");
        function.define("name", Property { accessor: false, value: JsValue::str(format!("bound {target_name}")), writable: false, enumerable: false, configurable: true, getter: None, setter: None }).unwrap();
        function.define("length", Property { accessor: false, value: JsValue::num((target_length - bound_count as f64).max(0.0)), writable: false, enumerable: false, configurable: true, getter: None, setter: None }).unwrap();
    }
    Ok(bound)
}

fn native_bound(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let function = i.native_stack.last().cloned().map(JsValue::Object)
        .ok_or_else(|| Throw::type_error("missing bound function"))?;
    i.invoke_bound_function(&function, args, i.current_new_target())
}

fn namespace_define_allowed(
    i: &mut Interpreter, target: &Obj, key: &PropertyKey, descriptor: &Obj,
) -> Result<bool, Throw> {
    let fields=i.std_descriptor_fields(&JsValue::Object(descriptor.clone()))?;
    namespace_define_fields(i,target,key,&fields)
}

fn native_define_property(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let target = match args.first() {
        Some(JsValue::Object(object)) => object.clone(),
        _ => return Err(Throw::type_error("defineProperty target is not an object")),
    };
    let key = i.to_property_key(args.get(1).unwrap_or(&JsValue::Undefined))?;
    let descriptor = args.get(2).cloned().unwrap_or(JsValue::Undefined);
    if !i.define_own_property(&target, &key, &descriptor)? {
        return Err(Throw::type_error("could not define property"));
    }

    Ok(JsValue::Object(target))
}

fn native_prevent_extensions(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let target = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let JsValue::Object(object) = &target {
        if !i.default_prevent_extensions(object)? {
            return Err(Throw::type_error("cannot prevent extensions on this object"));
        }
    }
    Ok(target)
}

fn native_is_extensible(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    match args.first() {
        Some(JsValue::Object(object)) => Ok(JsValue::Bool(i.default_is_extensible(object)?)),
        _ => Ok(JsValue::Bool(false)),
    }
}

fn native_date(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    intl_date_construct(i, this, args)
}

fn native_date_to_string(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    intl_date_to_string_method(i, this, args)
}

fn native_eval(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    i.perform_eval(args.first().cloned().unwrap_or(JsValue::Undefined))
}

fn native_error_value(i: &mut Interpreter, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let error = runtime_error_object(i, args.first().unwrap_or(&JsValue::Undefined))?;
    runtime_install_error_cause(i, &error, args.get(1).unwrap_or(&JsValue::Undefined))?;
    Ok(JsValue::Object(error))
}

fn native_error(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    native_error_value(i, args)
}

fn native_type_error(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    native_error_value(i, args)
}

fn native_eval_error(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    native_error_value(i, args)
}

fn native_uri_error(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    native_error_value(i, args)
}

fn native_range_error(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    native_error_value(i, args)
}

fn native_reference_error(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    native_error_value(i, args)
}

fn native_syntax_error(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    native_error_value(i, args)
}

fn native_error_to_string(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    if !this.is_object() {
        return Err(Throw::type_error("Error.prototype.toString called on non-object"));
    }
    let name = i.get_prop(&this, "name")?;
    let name = if name.is_undefined() { "Error".to_string() } else { i.to_string(&name)? };
    let message = i.get_prop(&this, "message")?;
    let message = if message.is_undefined() { String::new() } else { i.to_string(&message)? };
    let result = if name.is_empty() { message } else if message.is_empty() { name } else {
        format!("{name}: {message}")
    };
    Ok(JsValue::str(result))
}

// ES2026 Error.isError (§20.5.2.1). The [[ErrorData]] brand is the engine's
// Error exotic, which a writable JS property cannot forge; host platform
// errors (DOMException) are branded by the embedding via the same identity.
fn native_error_is_error(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let value = std_reflect_arg(&args, 0);
    let is_error = match &value {
        JsValue::Object(object) => {
            let object = object.borrow();
            matches!(object.exotic, Exotic::Error) || i.host_error_brand(&object)
        }
        _ => false,
    };
    Ok(JsValue::Bool(is_error))
}

// 9.2.7 %ThrowTypeError% (AddRestrictedFunctionProperties uses this shared,
// anonymous, non-constructible, length-0 intrinsic as both the getter and
// the setter installed for `Function.prototype.caller` / `.arguments`).
// Called for either accessor role, so it ignores `this`/args and just throws.
fn native_throw_type_error_intrinsic(_i: &mut Interpreter, _this: JsValue, _args: Vec<JsValue>) -> Result<JsValue, Throw> {
    Err(Throw::type_error(
        "'caller' and 'arguments' are restricted function properties and cannot be accessed in this context",
    ))
}

// JS-19 fix-before-push (2026-09-27): a plain SLOPPY function declaration/expression gets its
// OWN "caller" accessor (installed in `make_function`), shadowing the inherited
// %Function.prototype% poison pill above -- matching the non-standard but universally
// implemented "legacy function.caller" feature (test262 `features: [caller]`; V8 and
// SpiderMonkey both still ship it for web compat). It is NOT installed on strict, arrow,
// generator, async or method-defined functions, which keep the always-throwing inherited
// accessor (9.2.7 AddRestrictedFunctionProperties intends exactly that split: only "normal"
// sloppy functions get the historical, non-poisoned form).
//
// The historical [[Get]] rule this reproduces (matching real V8/SpiderMonkey, not the stricter
// ES5.1 15.3.5.4 draft text that would throw here): return the function's actual immediate
// caller, or `null` if it has none (top-level or direct-eval-at-top-level) or that caller is
// itself a strict-mode function -- a strict function's identity must never leak through a
// sloppy callee's `.caller`, but doing that by returning `null` rather than throwing is what
// `built-ins/Function/forbidden-ext`'s own generated cases assert (they read `inner.caller`
// unguarded, with no `try`/`catch`, so a throw here would fail them, not satisfy them).
//
// Receiver-driven, not distance-from-the-getter's-own-frame: a fixed relative offset (assuming
// exactly "getter, then reader, then caller" on top of `self.frames`) broke as soon as the read
// happened anywhere but a single bare `return f.caller;` statement -- an extra statement, an
// intervening `||`'s second operand, or a harness wrapper like `assert.throws` calling through
// another function first all shift how many frames sit between this getter's own call and the
// reader's, without changing which invocation is actually being asked about. Searching for the
// topmost still-active frame whose `own_function` IS `this` (by identity, `Rc::ptr_eq`) finds
// that invocation directly regardless of how many frames sit above or below it, and the frame
// one below THAT is its immediate caller.
fn native_legacy_function_caller_getter(i: &mut Interpreter, this: JsValue, _args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let JsValue::Object(reader) = &this else { return Ok(JsValue::Null) };
    let reader_index = i.frames.iter().enumerate().rev().find_map(|(index, frame)| {
        frame.own_function.as_ref().filter(|own| Rc::ptr_eq(&own.0, &reader.0)).map(|_| index)
    });
    let caller = match reader_index {
        Some(0) | None => None,
        Some(index) => i.frames[index - 1].own_function.clone(),
    };
    match caller {
        Some(caller) if !caller.borrow().strict_function => Ok(JsValue::Object(caller)),
        _ => Ok(JsValue::Null),
    }
}

// V8/SpiderMonkey silently ignore an assignment to a sloppy function's own legacy `.caller`;
// there is no writable backing slot for it to update.
fn native_legacy_function_caller_setter(_i: &mut Interpreter, _this: JsValue, _args: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(JsValue::Undefined)
}

// The `Error.prototype.stack` accessor (proposal "error-stack-accessor",
// pinned in this Test262 revision under built-ins/Error/prototype/stack/).
//
// get Error.prototype.stack ( ):
//   1. Let E be the this value. If E is not an Object, throw TypeError.
//   2. If E does not have an [[ErrorData]] internal slot, return undefined
//      (checked per-instance, not inherited -- Object.create(Error.prototype)
//      has Error.prototype on its chain but no [[ErrorData]] of its own).
//   3. Return an implementation-defined string; this host has no retained
//      call-frame metadata, so it formats the same header
//      Error.prototype.toString would produce.
fn native_error_stack_getter(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let JsValue::Object(object) = &this else {
        return Err(Throw::type_error("Error.prototype.stack getter called on a non-object"));
    };
    let has_error_data = {
        let borrowed = object.borrow();
        matches!(borrowed.exotic, Exotic::Error) || i.host_error_brand(&borrowed)
    };
    if !has_error_data {
        return Ok(JsValue::Undefined);
    }
    let name = i.get_prop(&this, "name")?;
    let name = if name.is_undefined() { "Error".to_string() } else { i.to_string(&name)? };
    let message = i.get_prop(&this, "message")?;
    let message = if message.is_undefined() { String::new() } else { i.to_string(&message)? };
    let header = if name.is_empty() {
        message
    } else if message.is_empty() {
        name
    } else {
        format!("{name}: {message}")
    };
    Ok(JsValue::str(header))
}

// set Error.prototype.stack ( v ):
//   1. Let E be the this value. If E is not an Object, throw TypeError.
//   2. If v is not a String, throw TypeError (no ToString coercion).
//   3. Perform ? SetterThatIgnoresPrototypeProperties(E, %Error.prototype%, "stack", v):
//      a. If SameValue(E, home) -- the receiver IS %Error.prototype% itself --
//         throw TypeError (emulates assigning a non-writable data property on
//         the home object).
//      b. Let desc be E.[[GetOwnProperty]]("stack") (own only).
//      c. If desc is undefined, CreateDataPropertyOrThrow(E, "stack", v): a
//         writable/enumerable/configurable data property, throwing if E is
//         non-extensible.
//      d. Else perform Set(E, "stack", v, true): an own data property keeps
//         its attributes and only its value changes (or throws if
//         non-writable); an own accessor calls its setter, or throws
//         TypeError if it has none.
fn native_error_stack_setter(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let JsValue::Object(object) = &this else {
        return Err(Throw::type_error("Error.prototype.stack setter called on a non-object"));
    };
    let value = args.into_iter().next().unwrap_or(JsValue::Undefined);
    if !matches!(value, JsValue::Str(_)) {
        return Err(Throw::type_error("Error.prototype.stack setter requires a String"));
    }
    if Rc::ptr_eq(&object.0, &i.error_proto.0) {
        return Err(Throw::type_error("cannot set \"stack\" on Error.prototype"));
    }
    let existing = object.borrow().own_prop("stack").cloned();
    match existing {
        None => {
            if !object.borrow().extensible {
                return Err(Throw::type_error("cannot create property \"stack\" on a non-extensible object"));
            }
            object.borrow_mut().define("stack", Property {
                accessor: false,
                value, writable: true, enumerable: true, configurable: true,
                getter: None, setter: None,
            }).unwrap();
        }
        Some(descriptor) => {
            if let Some(setter) = descriptor.setter.clone() {
                i.call_value(&setter, this.clone(), vec![value])?;
            } else if descriptor.is_accessor() {
                return Err(Throw::type_error("\"stack\" has no setter"));
            } else if !descriptor.writable {
                return Err(Throw::type_error("cannot assign to read only property \"stack\""));
            } else {
                let mut updated = descriptor;
                updated.value = value;
                object.borrow_mut().define("stack", updated).unwrap();
            }
        }
    }
    Ok(JsValue::Undefined)
}

fn new_promise(i: &Interpreter, state: PromiseFate) -> JsValue {
    let promise = Obj(Rc::new(RefCell::new({
        let mut o = JsObject::ordinary();
        o.exotic = Exotic::Promise(PromiseState {
            state,
            ..PromiseState::default()
        });
        o.proto = Some(i.promise_proto.clone());
        o
    })));
    JsValue::Object(promise)
}

fn promise_object(value: &JsValue) -> Result<Obj, Throw> {
    match value {
        JsValue::Object(o) if matches!(o.borrow().exotic, Exotic::Promise(_)) => Ok(o.clone()),
        _ => Err(Throw::type_error(
            "Promise method called on a non-Promise object",
        )),
    }
}

fn callable_function(value: &JsValue) -> Option<JsFunction> {
    match value {
        JsValue::Object(o) => o.borrow().func.clone(),
        _ => None,
    }
}

fn promise_callback(
    i: &Interpreter,
    name: &str,
    target: &Obj,
    f: crate::value::NativeFn,
) -> JsValue {
    let value = Interpreter::native_arity(name, f, &i.function_proto, 1);
    if let JsValue::Object(o) = &value {
        o.borrow_mut().promise_target = Some(target.clone());
    }
    value
}

fn native_promise(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let new_target = i.current_new_target().ok_or_else(|| Throw::type_error("Promise requires new"))?;
    let executor = args.first().cloned().unwrap_or(JsValue::Undefined);
    if callable_function(&executor).is_none() {
        return Err(Throw::type_error("Promise resolver is not a function"));
    }
    // OrdinaryCreateFromConstructor follows executor validation, and gets
    // prototype exactly once. Only a non-object prototype asks GetFunctionRealm
    // for the newTarget realm's immutable intrinsic fallback.
    let prototype = match i.get_prop(&new_target, "prototype")? {
        JsValue::Object(prototype) => prototype,
        _ => {
            let constructor = JsValue::Object(i.native_stack.last().cloned()
                .ok_or_else(|| Throw::type_error("missing Promise constructor context"))?);
            i.constructor_fallback_prototype(&constructor, &new_target)?
        }
    };
    let promise = promise_object(&new_promise(i, PromiseFate::Pending))?;
    promise.borrow_mut().proto = Some(prototype);
    let (resolve, reject) = promise_resolving_functions(i, &promise);
    if let Err(Throw(value)) = i.call_value(&executor, JsValue::Undefined, vec![resolve, reject.clone()]) {
        i.call_value(&reject, JsValue::Undefined, vec![value])?;
    }
    Ok(JsValue::Object(promise))
}

fn current_promise_target(i: &Interpreter) -> Option<Obj> {
    let function = i.native_stack.last()?;
    function.borrow().promise_target.clone()
}

fn native_promise_resolve(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    if let Some(target) = current_promise_target(i) {
        if !claim_current_promise_resolution(i, &target) { return Ok(JsValue::Undefined); }
        let value = args.first().cloned().unwrap_or(JsValue::Undefined);
        resolve_promise_value(i, &target, value)?;
        return Ok(JsValue::Undefined);
    }
    let value = args.first().cloned().unwrap_or(JsValue::Undefined);
    promise_resolve_constructor(i, &this, value)
}

fn native_promise_reject(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    if let Some(target) = current_promise_target(i) {
        if !claim_current_promise_resolution(i, &target) { return Ok(JsValue::Undefined); }
        settle_promise(
            &target,
            PromiseFate::Rejected(args.first().cloned().unwrap_or(JsValue::Undefined)),
            i,
        )?;
        return Ok(JsValue::Undefined);
    }
    let capability = new_promise_capability(i, &this)?;
    finish_promise_capability(&capability,
        PromiseFate::Rejected(args.first().cloned().unwrap_or(JsValue::Undefined)), i)?;
    Ok(capability_promise(&capability))
}

fn settle_promise(target: &Obj, fate: PromiseFate, i: &mut Interpreter) -> Result<(), Throw> {
    if !matches!(target.borrow().exotic, Exotic::Promise(_)) {
        return finish_promise_capability(target, fate, i);
    }
    let reactions = {
        let mut object = target.borrow_mut();
        let state = match &mut object.exotic {
            Exotic::Promise(state) if matches!(state.state, PromiseFate::Pending) => state,
            _ => return Ok(()),
        };
        let fulfilled = matches!(fate, PromiseFate::Fulfilled(_));
        state.state = fate.clone();
        if fulfilled {
            std::mem::take(&mut state.fulfill_reactions)
        } else {
            std::mem::take(&mut state.reject_reactions)
        }
    };
    let mut first_error = None;
    for reaction in reactions {
        if let Err(error) = i.enqueue_promise_job(PromiseJob {
            reaction,
            fate: fate.clone(),
        }) {
            if first_error.is_none() { first_error = Some(error); }
        }
    }
    first_error.map_or(Ok(()), Err)
}

fn run_promise_reaction(reaction: PendingReaction, fate: PromiseFate, i: &mut Interpreter) -> Result<(), Throw> {
    let previous_source = std::mem::replace(&mut i.active_source_override, Some(reaction.source_context));
    let result = match (reaction.handler, fate) {
        (Some(handler), PromiseFate::Fulfilled(value)) => {
            match i.call_value(&handler, JsValue::Undefined, vec![value]) {
                Ok(value) => PromiseFate::Fulfilled(value),
                Err(Throw(value)) => PromiseFate::Rejected(value),
            }
        }
        (Some(handler), PromiseFate::Rejected(value)) if reaction.is_reject_handler => {
            match i.call_value(&handler, JsValue::Undefined, vec![value]) {
                Ok(value) => PromiseFate::Fulfilled(value),
                Err(Throw(value)) => PromiseFate::Rejected(value),
            }
        }
        (Some(_), PromiseFate::Rejected(value)) => PromiseFate::Rejected(value),
        (None, PromiseFate::Fulfilled(value)) => PromiseFate::Fulfilled(value),
        (None, PromiseFate::Rejected(value)) => PromiseFate::Rejected(value),
        (_, PromiseFate::Pending) => PromiseFate::Pending,
    };
    i.active_source_override = previous_source;
    let Some(capability) = reaction.capability else {
        // Internal await handlers resume their suspended body and have no
        // chained promise to resolve. Surface an unexpected host abrupt
        // completion instead of creating or settling an observable promise.
        return match result {
            PromiseFate::Rejected(value) => Err(Throw(value)),
            _ => Ok(()),
        };
    };
    if matches!(capability.borrow().exotic, Exotic::Promise(_)) {
      if let PromiseFate::Fulfilled(value) = &result {
        if adopt_promise_result(&capability, value, i)? {
            return Ok(());
        }
      }
    }
    settle_promise(&capability, result, i)
}

fn adopt_promise_result(capability: &Obj, value: &JsValue, i: &mut Interpreter) -> Result<bool, Throw> {
    let Ok(source) = promise_object(value) else {
        return Ok(false);
    };
    if Rc::ptr_eq(&source.0, &capability.0) {
        settle_promise(
            capability,
            PromiseFate::Rejected(i.normalize_throw(Throw::type_error("promise resolution cycle")).0),
            i,
        )?;
        return Ok(true);
    }
    let state = match &source.borrow().exotic {
        Exotic::Promise(state) => state.state.clone(),
        _ => return Ok(false),
    };
    match state {
        PromiseFate::Pending => {
            let mut object = source.borrow_mut();
            if let Exotic::Promise(state) = &mut object.exotic {
                state.fulfill_reactions.push(PendingReaction {
                    handler: None,
                    source_context: None,
                    capability: Some(capability.clone()),
                    is_reject_handler: false,
                });
                state.reject_reactions.push(PendingReaction {
                    handler: None,
                    source_context: None,
                    capability: Some(capability.clone()),
                    is_reject_handler: true,
                });
            }
        }
        PromiseFate::Fulfilled(value) => i.enqueue_promise_job(PromiseJob {
            reaction: PendingReaction {
                handler: None,
                source_context: None,
                capability: Some(capability.clone()),
                is_reject_handler: false,
            },
            fate: PromiseFate::Fulfilled(value),
        })?,
        PromiseFate::Rejected(value) => i.enqueue_promise_job(PromiseJob {
            reaction: PendingReaction {
                handler: None,
                source_context: None,
                capability: Some(capability.clone()),
                is_reject_handler: true,
            },
            fate: PromiseFate::Rejected(value),
        })?,
    }
    Ok(true)
}

fn native_promise_then(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let promise = promise_object(&this)?;
    let constructor = promise_species_constructor(i, &this)?;
    let result = new_promise_capability(i, &constructor)?;
    perform_promise_then(i, &promise, args.first().cloned(), args.get(1).cloned(), Some(result.clone()))?;
    Ok(capability_promise(&result))
}

fn native_promise_catch(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let then = i.get_prop(&this, "then")?;
    i.call_value(&then, this, vec![
            JsValue::Undefined,
            args.first().cloned().unwrap_or(JsValue::Undefined),
        ])
}

fn native_data(i: &Interpreter) -> Result<Obj, Throw> {
    i.native_stack
        .last()
        .and_then(|function| function.borrow().func.as_ref()?.home.clone())
        .ok_or_else(|| Throw::type_error("missing native Promise callback state"))
}

fn combinator_state(i: &Interpreter) -> Result<(Obj, Obj, usize), Throw> {
    let data = native_data(i)?;
    let state = match data.borrow().get_own("state") {
        Some(JsValue::Object(state)) => state,
        _ => return Err(Throw::type_error("invalid Promise combinator state")),
    };
    let index = match data.borrow().get_own("index") {
        Some(JsValue::Num(index)) if index >= 0.0 => index as usize,
        _ => return Err(Throw::type_error("invalid Promise combinator index")),
    };
    let target = match state.borrow().get_own("target") {
        Some(JsValue::Object(target)) => target,
        _ => return Err(Throw::type_error("invalid Promise combinator target")),
    };
    Ok((state, target, index))
}

fn state_values(state: &Obj, name: &str) -> Result<JsValue, Throw> {
    state
        .borrow()
        .get_own(name)
        .ok_or_else(|| Throw::type_error("invalid Promise combinator values"))
}

fn state_remaining(state: &Obj) -> Result<usize, Throw> {
    match state.borrow().get_own("remaining") {
        Some(JsValue::Num(value)) if value >= 0.0 => Ok(value as usize),
        _ => Err(Throw::type_error("invalid Promise combinator count")),
    }
}

fn decrement_remaining(state: &Obj) -> Result<usize, Throw> {
    let remaining = state_remaining(state)?;
    let next = remaining.saturating_sub(1);
    state
        .borrow_mut()
        .set_own("remaining", JsValue::num(next as f64));
    Ok(next)
}

fn settled_record(i: &Interpreter, status: &str, key: &str, value: JsValue) -> JsValue {
    let record = i.object_value();
    if let JsValue::Object(object) = &record {
        object.borrow_mut().set_own("status", JsValue::str(status));
        object.borrow_mut().set_own(key, value);
    }
    record
}

fn make_combinator_state(target: &Obj, count: usize, values: JsValue) -> Obj {
    let state = Obj(Rc::new(RefCell::new(JsObject::ordinary())));
    state
        .borrow_mut()
        .set_own("target", JsValue::Object(target.clone()));
    state
        .borrow_mut()
        .set_own("remaining", JsValue::num(count as f64));
    state.borrow_mut().set_own("values", values);
    state
}

fn map_key(value: &JsValue) -> MapKey {
    match value {
        JsValue::Undefined => MapKey::Undefined,
        JsValue::Null => MapKey::Null,
        JsValue::Bool(v) => MapKey::Bool(*v),
        JsValue::Num(v) => MapKey::Num(*v),
        JsValue::BigInt(v) => MapKey::BigInt(v.clone()),
        JsValue::Str(v) => MapKey::Str(v.clone()),
        JsValue::Symbol(v) => MapKey::Symbol(v.clone()),
        JsValue::Object(v) => MapKey::Object(v.clone()),
    }
}

fn require_map(value: &JsValue) -> Result<Obj, Throw> {
    match value {
        JsValue::Object(o) if matches!(o.borrow().exotic, Exotic::Map(_)) => Ok(o.clone()),
        _ => Err(Throw::type_error("Map method called on a non-Map object")),
    }
}

fn require_set(value: &JsValue) -> Result<Obj, Throw> {
    match value {
        JsValue::Object(o) if matches!(o.borrow().exotic, Exotic::Set(_)) => Ok(o.clone()),
        _ => Err(Throw::type_error("Set method called on a non-Set object")),
    }
}

fn array_values(i: &mut Interpreter, values: Vec<JsValue>) -> Result<JsValue, Throw> {
    let out = JsValue::object({
        let mut o = JsObject::array();
        o.proto = Some(i.array_proto.clone());
        o
    });
    for (n, value) in values.into_iter().enumerate() {
        i.set_prop(&out, &n.to_string(), value)?;
    }
    Ok(out)
}

fn key_value(key: &MapKey) -> JsValue {
    match key {
        MapKey::Undefined => JsValue::Undefined,
        MapKey::Null => JsValue::Null,
        MapKey::Bool(v) => JsValue::Bool(*v),
        MapKey::Num(v) => JsValue::Num(*v),
        MapKey::BigInt(v) => JsValue::BigInt(v.clone()),
        MapKey::Str(v) => JsValue::Str(v.clone()),
        MapKey::Symbol(v) => JsValue::Symbol(v.clone()),
        MapKey::Object(v) => JsValue::Object(v.clone()),
    }
}

fn native_object_keys(i: &mut Interpreter, _: JsValue, a: Vec<JsValue>) -> Result<JsValue, Throw> {
    let src = a.first().cloned().unwrap_or(JsValue::Undefined);
    if src.is_null() || src.is_undefined() { return Err(Throw::type_error("Object.keys target is nullish")); }
    let src = native_object(i, JsValue::Undefined, vec![src])?;
    let keys = i
        .enumerable_keys(&src)?
        .into_iter()
        .map(JsValue::str)
        .collect();
    array_values(i, keys)
}

fn native_object_get_own_property_names(
    i: &mut Interpreter, _: JsValue, args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let value = args.first().unwrap_or(&JsValue::Undefined);
    if matches!(value, JsValue::Undefined | JsValue::Null) {
        return Err(Throw::type_error("cannot get property names of nullish value"));
    }
    let keys = i.own_property_keys(value)?.into_iter().filter_map(|key| match key {
        PropertyKey::String(name) => Some(JsValue::Str(name)), PropertyKey::Symbol(_) => None,
    }).collect();
    array_values(i, keys)
}

fn native_object_get_own_property_symbols(
    i: &mut Interpreter, _: JsValue, args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let value = args.first().unwrap_or(&JsValue::Undefined);
    if matches!(value, JsValue::Undefined | JsValue::Null) {
        return Err(Throw::type_error("cannot get symbols of nullish value"));
    }
    let keys = i.own_property_keys(value)?.into_iter().filter_map(|key| match key {
        PropertyKey::Symbol(symbol) => Some(JsValue::Symbol(symbol)), PropertyKey::String(_) => None,
    }).collect();
    array_values(i, keys)
}

fn native_object_get_own_property_descriptor(
    i: &mut Interpreter, _: JsValue, args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let Some(target) = args.first() else { return Err(Throw::type_error("missing object")); };
    if target.is_null() || target.is_undefined() {
        return Err(Throw::type_error("property descriptor target is nullish"));
    }
    let key = i.to_property_key(args.get(1).unwrap_or(&JsValue::Undefined))?;
    let JsValue::Object(object) = target else {
        let property = match target {
            JsValue::Str(string) if key == PropertyKey::from("length") => Some(Property {
                accessor: false,
                value: JsValue::num(string.len() as f64), writable: false,
                enumerable: false, configurable: false, getter: None, setter: None,
            }),
            JsValue::Str(string) => array_index_key(&key)
                .and_then(|index| string.units().get(index as usize).copied())
                .map(|unit| Property {
                    accessor: false,
                    value: JsValue::Str(crate::value::JsString::from_units(vec![unit])), writable: false,
                    enumerable: true, configurable: false, getter: None, setter: None,
                }),
            _ => None,
        };
        return Ok(property.map(|property| object_descriptor_value(i, property))
            .unwrap_or(JsValue::Undefined));
    };
    if let Some(result) = i.window_proxy_own_property(object, &key) {
        return Ok(result?.map(|property| object_descriptor_value(i, property)).unwrap_or(JsValue::Undefined));
    }
    if matches!(object.borrow().exotic, Exotic::Proxy(_)) {
        return Ok(i.default_get_own_property(object, &key)?
            .map(|property| object_descriptor_value(i, property)).unwrap_or(JsValue::Undefined));
    }
    if let Some(property) = binary_index_descriptor(object, &key) {
        let Some(property) = property else { return Ok(JsValue::Undefined); };
        let mut descriptor = JsObject::ordinary();
        descriptor.proto = Some(i.object_proto.clone());
        descriptor.set_own("value", property.value);
        descriptor.set_own("writable", JsValue::Bool(true));
        descriptor.set_own("enumerable", JsValue::Bool(true));
        descriptor.set_own("configurable", JsValue::Bool(true));
        return Ok(JsValue::object(descriptor));
    }
    if let Some(result)=i.namespace_own_property(object,&key) {
        return Ok(result?.map(|property|object_descriptor_value(i,property)).unwrap_or(JsValue::Undefined));
    }
    let property = if let Some(result) = arguments_own_property(object, &key) { result? } else { match &key {
        PropertyKey::String(name) => i.legacy_own_property(object, name, false)?,
        PropertyKey::Symbol(_) => object.borrow().own_prop(&key).cloned(),
    } };
    Ok(property.map(|property| object_descriptor_value(i, property))
        .unwrap_or(JsValue::Undefined))
}

fn object_descriptor_value(i: &Interpreter, property: Property) -> JsValue {
    let mut descriptor = JsObject::ordinary();
    descriptor.proto = Some(i.object_proto.clone());
    descriptor.set_own("enumerable", JsValue::Bool(property.enumerable));
    descriptor.set_own("configurable", JsValue::Bool(property.configurable));
    if property.is_accessor() {
        descriptor.set_own("get", property.getter.unwrap_or(JsValue::Undefined));
        descriptor.set_own("set", property.setter.unwrap_or(JsValue::Undefined));
    } else {
        descriptor.set_own("value", property.value);
        descriptor.set_own("writable", JsValue::Bool(property.writable));
    }
    JsValue::object(descriptor)
}

fn native_reflect_own_keys(
    i: &mut Interpreter, _: JsValue, args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let Some(JsValue::Object(object)) = args.first() else { return Err(Throw::type_error("Reflect.ownKeys requires an object")); };
    let keys = i.own_property_keys(&JsValue::Object(object.clone()))?.into_iter().map(|key| match key {
        PropertyKey::String(name) => JsValue::Str(name), PropertyKey::Symbol(symbol) => JsValue::Symbol(symbol),
    }).collect();
    array_values(i, keys)
}

fn reflect_target(args: &[JsValue]) -> Result<Obj, Throw> {
    match args.first() {
        Some(JsValue::Object(object)) => Ok(object.clone()),
        _ => Err(Throw::type_error("Reflect target is not an object")),
    }
}

fn native_reflect_get(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let target = reflect_target(&args)?;
    let key = i.to_property_key(args.get(1).unwrap_or(&JsValue::Undefined))?;
    let receiver = args.get(2).cloned().unwrap_or_else(|| JsValue::Object(target.clone()));
    i.get_prop_receiver(&JsValue::Object(target), &key, receiver)
}

fn native_reflect_set(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let target = reflect_target(&args)?;
    if is_module_namespace(&target) {
        return Ok(JsValue::Bool(false));
    }
    let key = i.to_property_key(args.get(1).unwrap_or(&JsValue::Undefined))?;
    let value = args.get(2).cloned().unwrap_or(JsValue::Undefined);
    let receiver = args.get(3).cloned().unwrap_or_else(|| JsValue::Object(target.clone()));
    Ok(JsValue::Bool(i.set_with_receiver(&target, &key, value, receiver)?))
}

fn native_reflect_has(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let target = reflect_target(&args)?;
    let key = i.to_property_key(args.get(1).unwrap_or(&JsValue::Undefined))?;
    if let Some(property) = binary_index_descriptor(&target, &key) { return Ok(JsValue::Bool(property.is_some())); }
    Ok(JsValue::Bool(match key.as_string() {
        Some(name) => i.legacy_has_property(&target, name)?,
        None => target.borrow().has_property(&key),
    }))
}

fn native_reflect_delete_property(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let target = reflect_target(&args)?;
    let key = i.to_property_key(args.get(1).unwrap_or(&JsValue::Undefined))?;
    if let Some(result) = i.window_proxy_delete(&target, &key) { return Ok(JsValue::Bool(result?)); }
    if is_module_namespace(&target) {
        return Ok(JsValue::Bool(target.borrow().own_prop(&key).is_none()));
    }
    if let Some(deleted) = binary_delete_index(&target, &key) { return Ok(JsValue::Bool(deleted)); }
    let deleted = match key.as_string() {
        Some(name) => match i.legacy_delete(&target, name)? {
            Some(value) => value, None => target.borrow_mut().delete_own(&key),
        },
        None => target.borrow_mut().delete_own(&key),
    };
    Ok(JsValue::Bool(deleted))
}

fn native_reflect_define_property(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let target = reflect_target(&args)?;
    let key = i.to_property_key(args.get(1).unwrap_or(&JsValue::Undefined))?;
    let descriptor = args.get(2).cloned().unwrap_or(JsValue::Undefined);
    if let JsValue::Object(object) = &descriptor {
        if let Some(result) = binary_define_index_object(i, &target, &key, object) {
            return Ok(JsValue::Bool(result?));
        }
    }
    Ok(JsValue::Bool(i.define_own_property(&target, &key, &descriptor)?))
}

fn native_reflect_construct(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let target = args.first().cloned().unwrap_or(JsValue::Undefined);
    let new_target = args.get(2).cloned().unwrap_or_else(|| target.clone());
    for constructor in [&target, &new_target] {
        let JsValue::Object(object) = constructor else {
            return Err(Throw::type_error("Reflect.construct requires constructors"));
        };
        let object = object.borrow();
        if object.func.is_none() || !object.constructible {
            return Err(Throw::type_error("Reflect.construct requires constructors"));
        }
    }
    let arguments = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    let values = i.create_list_from_array_like(&arguments)?;
    i.construct_with_new_target(&target, values, new_target)
}

fn native_object_values(
    i: &mut Interpreter,
    _: JsValue,
    a: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let source = a.first().cloned().unwrap_or(JsValue::Undefined);
    if source.is_null() || source.is_undefined() {
        return Err(Throw::type_error("Object.values requires an object"));
    }
    let mut values = Vec::new();
    for key in i.own_property_keys(&source)? {
        if matches!(key, PropertyKey::String(_)) && i.object_own_enumerable(&source, &key)? {
            values.push(i.get_prop(&source, &key)?);
        }
    }
    array_values(i, values)
}

fn native_object_assign(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let input = args.first().cloned().unwrap_or(JsValue::Undefined);
    if input.is_null() || input.is_undefined() {
        return Err(Throw::type_error("Object.assign target is nullish"));
    }
    let target = native_object(i, JsValue::Undefined, vec![input])?;
    let JsValue::Object(object) = &target else { unreachable!() };
    for source in args.iter().skip(1) {
        for key in i.own_property_keys(source)? {
            if !i.object_own_enumerable(source, &key)? { continue; }
            let value = i.get_prop(source, &key)?;
            if object.borrow().boxed_primitive.clone().and_then(|data| match data {
                JsValue::Str(string) => Some(string.len()), _ => None,
            }).is_some_and(|length| array_index_key(&key).is_some_and(|index| (index as usize) < length)) {
                return Err(Throw::type_error("Object.assign cannot write a String index"));
            }
            if !i.set_with_receiver(object, &key, value, target.clone())? {
                return Err(Throw::type_error("Object.assign could not set property"));
            }
        }
    }
    Ok(target)
}

fn native_json_parse(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let source = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    // JSONWhiteSpace is narrower than ECMAScript WhiteSpace. The script
    // parser below accepts additional Unicode separators, so reject them
    // outside quoted JSON strings before handing the text to it.
    let mut in_string = false;
    let mut escaped = false;
    for ch in source.chars() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
        } else if ch == '"' {
            in_string = true;

        } else if ch.is_whitespace() && !matches!(ch, ' ' | '\t' | '\n' | '\r') {
            return Err(Throw::syntax_error("invalid JSON whitespace"));
        }
    }
    let program = crate::parse_script(&format!("({source})"))
        .map_err(|error| Throw::syntax_error(error.to_string()))?;
    i.run_program(&program)
}

fn json_escape(value: &crate::value::JsString) -> String {
    let mut output = String::from("\"");
    let mut units = value.units().iter().copied().peekable();
    while let Some(unit) = units.next() {
        match unit {
            0x22 => output.push_str("\\\""),
            0x5c => output.push_str("\\\\"),
            0x0a => output.push_str("\\n"),
            0x0d => output.push_str("\\r"),
            0x09 => output.push_str("\\t"),
            0x00..=0x1f | 0xd800..=0xdfff => {
                if (0xd800..=0xdbff).contains(&unit)
                    && units.peek().is_some_and(|next| (0xdc00..=0xdfff).contains(next)) {
                    let low = units.next().unwrap_or(0xdc00);
                    let point = 0x10000 + (((unit as u32 - 0xd800) << 10) | (low as u32 - 0xdc00));
                    if let Some(ch) = char::from_u32(point) { output.push(ch); }
                } else {
                    output.push_str(&format!("\\u{unit:04x}"));
                }
            }
            _ => if let Some(ch) = char::from_u32(unit as u32) { output.push(ch); },
        }
    }
    output.push('"');
    output
}

fn json_stringify_value(i: &mut Interpreter, value: &JsValue) -> Result<Option<String>, Throw> {
    Ok(match value {
        JsValue::Undefined | JsValue::Symbol(_) => None,
        JsValue::Null => Some("null".into()),
        JsValue::Bool(value) => Some(value.to_string()),
        JsValue::Num(value) if value.is_finite() => Some(crate::value::number_to_string(*value)),
        JsValue::Num(_) => Some("null".into()),
        JsValue::BigInt(_) => return Err(Throw::type_error("BigInt is not JSON serializable")),
        JsValue::Str(value) => Some(json_escape(value)),
        JsValue::Object(object) if object.borrow().func.is_some() => None,
        JsValue::Object(object) if matches!(object.borrow().exotic, Exotic::Array) => {
            let mut parts = Vec::new();
            for index in 0..i.array_length(value) {
                let element = i.get_prop(value, &index.to_string())?;
                parts.push(json_stringify_value(i, &element)?.unwrap_or_else(|| "null".into()));
            }
            Some(format!("[{}]", parts.join(",")))
        }
        JsValue::Object(_) => {
            let mut parts = Vec::new();
            for key in i.enumerable_keys(value)? {
                let property = i.get_prop(value, &key)?;
                if let Some(value) = json_stringify_value(i, &property)? {
                    parts.push(format!("{}:{}", json_escape(&key), value));
                }
            }
            Some(format!("{{{}}}", parts.join(",")))
        }
    })
}

fn native_json_stringify(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let value = args.first().unwrap_or(&JsValue::Undefined);
    Ok(json_stringify_value(i, value)?
        .map(JsValue::str)
        .unwrap_or(JsValue::Undefined))
}

fn native_parse_int(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let mut source = i
        .to_string(args.first().unwrap_or(&JsValue::Undefined))?
        .trim()
        .to_string();
    let negative = source.starts_with('-');
    if negative || source.starts_with('+') {
        source.remove(0);
    }
    let mut radix = i.to_number(args.get(1).unwrap_or(&JsValue::num(0.0)))? as i32;
    if radix == 0 {
        radix = if source.starts_with("0x") || source.starts_with("0X") {
            16
        } else {
            10
        };
    }
    if !(2..=36).contains(&radix) {
        return Ok(JsValue::num(f64::NAN));
    }
    if radix == 16 && (source.starts_with("0x") || source.starts_with("0X")) {
        source.drain(..2);
    }
    let digits: String = source
        .chars()
        .take_while(|ch| ch.to_digit(radix as u32).is_some())
        .collect();
    if digits.is_empty() {
        return Ok(JsValue::num(f64::NAN));
    }
    let number = u128::from_str_radix(&digits, radix as u32).unwrap_or(0) as f64;
    Ok(JsValue::num(if negative { -number } else { number }))
}

fn native_parse_float(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let source = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    let trimmed = source.trim();
    if trimmed.starts_with("Infinity") {
        return Ok(JsValue::num(f64::INFINITY));
    }
    let mut end = 0usize;
    for (index, ch) in trimmed.char_indices() {
        if ch.is_ascii_digit() || matches!(ch, '+' | '-' | '.' | 'e' | 'E') {
            end = index + ch.len_utf8();
        } else {
            break;
        }
    }
    Ok(JsValue::num(
        trimmed[..end].parse::<f64>().unwrap_or(f64::NAN),
    ))
}

fn native_is_finite(_: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    match args.first().unwrap_or(&JsValue::Undefined) {
        JsValue::Num(value) => Ok(JsValue::Bool(value.is_finite())),
        _ => Ok(JsValue::Bool(false)),
    }
}
fn native_object_to_string(
    i: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    if matches!(this, JsValue::Undefined | JsValue::Null) {
        return Ok(JsValue::str(if this.is_undefined() { "[object Undefined]" } else { "[object Null]" }));
    }
    let boxed = std_legacy_to_object(i, this)?;
    let JsValue::Object(object) = &boxed else { unreachable!() };
    // IsArray must run before the observable @@toStringTag Get.
    let base = if is_array_value(i, &boxed)? { "Array" } else {
        let object = object.borrow();
        if object.arguments_object { "Arguments" }
        else if object.func.is_some() { "Function" }
        else if matches!(object.exotic, Exotic::Error) || i.host_error_brand(&object) { "Error" }
        else if matches!(object.boxed_primitive, Some(JsValue::Bool(_))) { "Boolean" }
        else if matches!(object.boxed_primitive, Some(JsValue::Num(_))) { "Number" }
        else if matches!(object.boxed_primitive, Some(JsValue::Str(_))) { "String" }
        else if !matches!(object.exotic, Exotic::Proxy(_)) && object.get_own("[[DateValue]]").is_some() { "Date" }
        else if matches!(object.exotic, Exotic::RegExp(_)) { "RegExp" }
        else { "Object" }
    };
    let tag = i.get_prop(&boxed, WellKnownSymbol::ToStringTag.symbol())?;
    let tag = match tag { JsValue::Str(value) => value, _ => base.into() };
    let mut units: Vec<u16> = "[object ".encode_utf16().collect();
    units.extend_from_slice(tag.units());
    units.push(b']' as u16);
    Ok(JsValue::str_units(units))
}
fn native_object_value_of(
    i: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    if matches!(this, JsValue::Null | JsValue::Undefined) {
        return Err(Throw::type_error("Object.prototype.valueOf called on null or undefined"));
    }
    native_object(i, JsValue::Undefined, vec![this])
}

thread_local! {
    // Cycle protection for Array.prototype.join/toString: `var a=[]; a.push(a);
    // a.join()` converts its own circular element via ToString(element),
    // which is [[Get]] "join"/[[Call]] on the SAME array again. Without a
    // guard that recurses without bound and overflows the real Rust call
    // stack (a process crash, not a catchable JS error); real engines break
    // the cycle by having the re-entrant call on a currently-joining array
    // contribute nothing, so `a.join()` above is `""`, matching observed
    // browser behavior. Keyed by object identity (std_weak_id), one stack per
    // thread since the interpreter is single-threaded per realm/agent.
    static ARRAY_JOIN_STACK: RefCell<Vec<usize>> = RefCell::new(Vec::new());
}

/// Pops its identity off ARRAY_JOIN_STACK on drop, so every early `?` return
/// inside native_array_join still unwinds the guard correctly.
struct ArrayJoinGuard(bool);
impl Drop for ArrayJoinGuard {
    fn drop(&mut self) {
        if self.0 { ARRAY_JOIN_STACK.with(|stack| { stack.borrow_mut().pop(); }); }
    }
}

fn native_array_join(
    i: &mut Interpreter,
    this: JsValue,
    a: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    if this.is_null() || this.is_undefined() {
        return Err(Throw::type_error("Array.prototype.join called on null or undefined"));
    }
    let this = array_receiver(i, this)?;
    let _guard = if let JsValue::Object(object) = &this {
        let id = std_weak_id(object);
        if ARRAY_JOIN_STACK.with(|stack| stack.borrow().contains(&id)) {
            return Ok(JsValue::str(String::new()));
        }
        ARRAY_JOIN_STACK.with(|stack| stack.borrow_mut().push(id));
        ArrayJoinGuard(true)
    } else {
        ArrayJoinGuard(false)
    };
    let length = i.get_prop(&this, "length")?;
    let length = i.to_number(&length)?;
    let length = if length.is_nan() || length <= 0.0 { 0 } else {
        length.floor().min(9_007_199_254_740_991.0) as usize
    };
    let sep = if let Some(v) = a.first().filter(|v| !v.is_undefined()) {
        i.to_string(v)?
    } else {
        ",".to_string()
    };
    let mut out = String::new();
    for n in 0..length {
        if n > 0 {
            out.push_str(&sep)
        }
        let v = i.get_prop(&this, &n.to_string())?;
        if !v.is_null() && !v.is_undefined() {
            out.push_str(&i.to_string(&v)?);
        }
    }
    Ok(JsValue::str(out))
}
fn native_array_to_string(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    if this.is_null() || this.is_undefined() {
        return Err(Throw::type_error("Array.prototype.toString called on null or undefined"));
    }
    let this = array_receiver(i, this)?;
    let join = i.get_prop(&this, "join")?;
    if matches!(&join, JsValue::Object(object) if object.borrow().func.is_some()) {
        i.call_value(&join, this, Vec::new())
    } else {
        native_object_to_string(i, this, Vec::new())
    }
}
fn native_array(i: &mut Interpreter, _: JsValue, a: Vec<JsValue>) -> Result<JsValue, Throw> {
    let v = JsValue::object({
        let mut o = JsObject::array();
        o.proto = Some(i.array_proto.clone());
        o
    });
    if let [JsValue::Num(length)] = a.as_slice() {
        if !length.is_finite() || *length < 0.0 || length.fract() != 0.0 || *length > u32::MAX as f64 {
            return Err(Throw::range_error("invalid array length"));
        }
        i.set_prop(&v, "length", JsValue::num(*length))?;
        return Ok(v);
    }
    for (n, x) in a.into_iter().enumerate() {
        i.set_prop(&v, &n.to_string(), x)?;
    }
    Ok(v)
}
fn native_abs(i: &mut Interpreter, _: JsValue, a: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(JsValue::num(
        i.to_number(a.first().unwrap_or(&JsValue::Undefined))?.abs(),
    ))
}
fn math_arg(i: &mut Interpreter, args: &[JsValue]) -> Result<f64, Throw> {
    i.to_number(args.first().unwrap_or(&JsValue::Undefined))
}
fn native_math_floor(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    Ok(JsValue::num(math_arg(i, &args)?.floor()))
}
fn native_math_ceil(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(JsValue::num(math_arg(i, &args)?.ceil()))
}
fn native_math_round(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let value = math_arg(i, &args)?;
    Ok(JsValue::num(if value.fract() == -0.5 {
        value.ceil()
    } else {
        value.round()
    }))
}
fn native_math_sqrt(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(JsValue::num(math_arg(i, &args)?.sqrt()))
}
fn native_math_pow(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let base = i.to_number(args.first().unwrap_or(&JsValue::Undefined))?;
    let exponent = i.to_number(args.get(1).unwrap_or(&JsValue::Undefined))?;
    Ok(JsValue::num(crate::value::number_exponentiate(base, exponent)))
}
fn native_math_trunc(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    Ok(JsValue::num(math_arg(i, &args)?.trunc()))
}
fn native_math_sign(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let value = math_arg(i, &args)?;
    Ok(JsValue::num(if value.is_nan() {
        f64::NAN
    } else if value == 0.0 {
        value
    } else if value < 0.0 {
        -1.0
    } else {
        1.0
    }))
}
fn native_math_random(i: &mut Interpreter, _: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    // SplitMix64 gives each interpreter a reproducible sequence with a full-width seed.
    i.random_state = i.random_state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut bits = i.random_state;
    bits = (bits ^ (bits >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    bits = (bits ^ (bits >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    bits ^= bits >> 31;
    Ok(JsValue::num(((bits >> 11) as f64) * (1.0 / 9_007_199_254_740_992.0)))
}
fn native_max(i: &mut Interpreter, _: JsValue, a: Vec<JsValue>) -> Result<JsValue, Throw> {
    let mut n = f64::NEG_INFINITY;
    for x in &a {
        n = n.max(i.to_number(x)?);
    }
    Ok(JsValue::num(n))
}
fn native_min(i: &mut Interpreter, _: JsValue, a: Vec<JsValue>) -> Result<JsValue, Throw> {
    let mut n = f64::INFINITY;
    for x in &a {
        n = n.min(i.to_number(x)?);
    }
    Ok(JsValue::num(n))
}
fn native_number(i: &mut Interpreter, this: JsValue, a: Vec<JsValue>) -> Result<JsValue, Throw> {
    let number = if let Some(argument) = a.first() {
        // Number(value) is ToNumeric(value) (21.1.1.1) -- ToPrimitive with the number
        // hint, not default.
        let primitive = i.primitive_number_hint(argument.clone())?;
        if let JsValue::BigInt(bigint) = &primitive { bigint.to_f64() }
        else { i.to_number(&primitive)? }
    } else { 0.0 };
    let value = JsValue::num(number);
    if let JsValue::Object(object) = &this {
        object.borrow_mut().boxed_primitive = Some(value);
        Ok(this)
    } else {
        Ok(value)
    }
}
// Required by testharness.js when building its control-character RegExp.
fn native_string_from_char_code(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let mut units = Vec::with_capacity(args.len());
    for arg in args {
        let number = i.to_number(&arg)?;
        units.push(if number.is_finite() {
            number.trunc().rem_euclid(65536.0) as u16
        } else {
            0
        });
    }
    Ok(JsValue::str_units(units))
}

fn native_string(i: &mut Interpreter, this: JsValue, a: Vec<JsValue>) -> Result<JsValue, Throw> {
    let constructing = i.current_new_target().is_some();
    let value = match a.first() {
        None => JsValue::str(""),
        Some(JsValue::Symbol(symbol)) if !constructing => JsValue::str_units(symbol.descriptive_units()),
        Some(value) => JsValue::str(i.to_js_string(value)?),
    };
    if constructing {
        let JsValue::Object(object) = &this else { unreachable!() };
        let JsValue::Str(string) = value else { unreachable!() };
        object.borrow_mut().initialize_string_data(string);
        Ok(this)
    } else {
        Ok(value)
    }
}

fn native_string_split(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    if this.is_null() || this.is_undefined() {
        return Err(Throw::type_error("String method called on null or undefined"));
    }
    let separator = args.first().cloned().unwrap_or(JsValue::Undefined);
    // Step 2: an object separator's @@split method takes precedence, if present.
    if separator.is_object() {
        let splitter = i.get_prop(&separator, WellKnownSymbol::Split.symbol())?;
        if !splitter.is_null() && !splitter.is_undefined() {
            if callable_function(&splitter).is_none() {
                return Err(Throw::type_error("@@split is not a function"));
            }
            let limit = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            return i.call_value(&splitter, separator, vec![this, limit]);
        }
    }
    let source = string_this_units(i, this)?;
    let lim = match args.get(1) {
        Some(value) if !value.is_undefined() => i.to_uint32(value)? as usize,
        _ => u32::MAX as usize,
    };
    // A RegExp separator is handled by @@split before ToString(separator).
    if let Some(regex) = regexp_data(&separator) {
        if lim == 0 {
            return array_values(i, Vec::new());
        }
        let units = regexp_units(&source)?;
        let values = regexp_split_units(&regex, &units, lim)?;
        return array_values(i, values);
    }
    // ToString(separator) precedes the limit check, so a throwing separator
    // toString wins even when limit is 0.
    let r = match separator.is_undefined() {
        true => None,
        false => Some(i.to_js_string(&separator)?),
    };
    if lim == 0 {
        return array_values(i, Vec::new());
    }
    let Some(r) = r else {
        return array_values(i, vec![JsValue::str(source)]);
    };
    if r.is_empty() {
        let mut values = Vec::new();
        for unit in source.units() {
            if values.len() == lim { break; }
            values.push(JsValue::str_units(vec![*unit]));
        }
        return array_values(i, values);
    }
    if source.is_empty() {
        return array_values(i, vec![JsValue::str(source)]);
    }
    string_split_by(i, &source, &r, lim)
}

// SplitMatch over a string separator: emit the text between matches, bounded by
// lim.
fn string_split_by(
    i: &mut Interpreter,
    source: &crate::value::JsString,
    separator: &crate::value::JsString,
    lim: usize,
) -> Result<JsValue, Throw> {
    let units = source.units();
    let separator = separator.units();
    let size = units.len();
    let mut values = Vec::new();
    let mut p = 0;
    let mut q = 0;
    while q < size {
        if q + separator.len() <= size && units[q..q + separator.len()] == *separator {
            values.push(JsValue::str_units(units[p..q].to_vec()));
            if values.len() == lim { return array_values(i, values); }
            p = q + separator.len();
            q = p;
        } else {
            q += 1;
        }
    }
    values.push(JsValue::str_units(units[p..size].to_vec()));
    array_values(i, values)
}

// RegExp.prototype[@@split] with lim already coerced. Matching is anchored at q
// by rejecting any match that starts later; advance_index handles empty matches.
fn regexp_split_units(
    regex: &Rc<amazing_regex::Regex>,
    units: &[u16],
    lim: usize,
) -> Result<Vec<JsValue>, Throw> {
    let size = units.len();
    let mut values = Vec::new();
    if lim == 0 {
        return Ok(values);
    }
    if size == 0 {
        if regex.exec_utf16(units, 0).map_err(regexp_error)?.is_some() {
            return Ok(values);
        }
        values.push(JsValue::str_units(units.to_vec()));
        return Ok(values);
    }
    let unicode = regex.flags().unicode;
    let mut p = 0;
    let mut q = 0;
    while q < size {
        let matched = match regex.exec_utf16(units, q).map_err(regexp_error)? {
            Some(m) if m.range.start == q => m,
            _ => {
                q = amazing_regex::advance_index(units, q, unicode);
                continue;
            }
        };
        let end = matched.range.end.min(size);
        if end == p {
            q = amazing_regex::advance_index(units, q, unicode);
            continue;
        }
        values.push(JsValue::str_units(units[p..q].to_vec()));
        if values.len() == lim { return Ok(values); }
        p = end;
        for capture in matched.captures.iter().skip(1) {
            values.push(match capture {
                Some(range) => JsValue::str_units(units[range.clone()].to_vec()),
                None => JsValue::Undefined,
            });
            if values.len() == lim { return Ok(values); }
        }
        q = p;
    }
    values.push(JsValue::str_units(units[p..size].to_vec()));
    Ok(values)
}

fn string_this_units(i: &mut Interpreter, this: JsValue) -> Result<crate::value::JsString, Throw> {
    if matches!(this, JsValue::Null | JsValue::Undefined) {
        return Err(Throw::type_error("String method called on null or undefined"));
    }
    i.to_js_string(&this)
}

fn native_string_value_of(_: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    match this {
        JsValue::Str(_) => Ok(this),
        JsValue::Object(object) => object.borrow().boxed_primitive.clone().filter(|v| matches!(v, JsValue::Str(_)))
            .ok_or_else(|| Throw::type_error("incompatible String receiver")),
        _ => Err(Throw::type_error("incompatible String receiver")),
    }
}

fn string_position(i: &mut Interpreter, value: Option<&JsValue>, default: f64, limit: usize) -> Result<usize, Throw> {
    let n = match value { Some(v) => i.to_number(v)?, None => default };
    if n.is_nan() || n <= 0.0 { Ok(0) }
    else if n >= limit as f64 { Ok(limit) }
    else { Ok(n.trunc() as usize) }
}

// ToIntegerOrInfinity: NaN and -0 become +0, infinities are preserved.
fn to_integer_or_infinity(i: &mut Interpreter, value: &JsValue) -> Result<f64, Throw> {
    let n = i.to_number(value)?;
    if n.is_nan() { Ok(0.0) } else if n.is_infinite() { Ok(n) } else { Ok(n.trunc()) }
}

// An out-of-range string index (negative or past the end) yields None; callers
// map that to "" / NaN / undefined as their spec requires.
fn string_index(i: &mut Interpreter, value: &JsValue, len: usize) -> Result<Option<usize>, Throw> {
    let n = to_integer_or_infinity(i, value)?;
    if n < 0.0 || n >= len as f64 { Ok(None) } else { Ok(Some(n as usize)) }
}

fn native_string_trim(
    i: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let source = string_this_units(i, this)?;
    let start = source.units().iter().position(|unit| !std_string_space(*unit)).unwrap_or(source.len());
    let end = source.units().iter().rposition(|unit| !std_string_space(*unit)).map_or(start, |i| i + 1);
    Ok(JsValue::str_units(source.units()[start..end].to_vec()))
}

fn native_string_char_at(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let source = string_this_units(i, this)?;
    let position = args.first().cloned().unwrap_or(JsValue::Undefined);
    let Some(index) = string_index(i, &position, source.len())? else {
        return Ok(JsValue::str(""));
    };
    Ok(JsValue::str_units(vec![source.units()[index]]))
}

fn native_string_char_code_at(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let source = string_this_units(i, this)?;
    let position = args.first().cloned().unwrap_or(JsValue::Undefined);
    let Some(index) = string_index(i, &position, source.len())? else {
        return Ok(JsValue::num(f64::NAN));
    };
    Ok(JsValue::num(source.units()[index] as f64))
}

fn native_string_code_point_at(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let source = string_this_units(i, this)?;
    let position = args.first().cloned().unwrap_or(JsValue::Undefined);
    let Some(index) = string_index(i, &position, source.len())? else {
        return Ok(JsValue::Undefined);
    };
    let first = source.units()[index];
    let point = if (0xd800..=0xdbff).contains(&first) {
        match source.units().get(index + 1) {
            Some(&second) if (0xdc00..=0xdfff).contains(&second) =>
                0x10000 + (((first as u32 - 0xd800) << 10) | (second as u32 - 0xdc00)),
            _ => first as u32,
        }
    } else { first as u32 };
    Ok(JsValue::num(point as f64))
}

fn native_string_slice(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let source = string_this_units(i, this)?;
    let len = source.len() as f64;
    let index = |n: f64| -> usize {
        if n.is_nan() { 0 } else if n < 0.0 { (len + n.trunc()).clamp(0.0, len) as usize }
        else { n.trunc().clamp(0.0, len) as usize }
    };
    let start = index(i.to_number(args.first().unwrap_or(&JsValue::Undefined))?);
    let end = match args.get(1) { Some(v) if !v.is_undefined() => index(i.to_number(v)?), _ => source.len() };
    Ok(JsValue::str_units(source.units()[start..end.max(start)].to_vec()))
}

fn native_string_substring(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let source = string_this_units(i, this)?;
    let start = string_position(i, args.first().filter(|value| !value.is_undefined()), 0.0, source.len())?;
    let end = string_position(i, args.get(1).filter(|value| !value.is_undefined()), source.len() as f64, source.len())?;
    let (a, b) = if start <= end { (start, end) } else { (end, start) };
    Ok(JsValue::str_units(source.units()[a..b].to_vec()))
}

fn native_string_index_of(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let source = string_this_units(i, this)?;
    let needle = i.to_js_string(args.first().unwrap_or(&JsValue::Undefined))?;
    let start = string_position(i, args.get(1), 0.0, source.len())?;
    let found = if needle.is_empty() { Some(start) } else {
        source.units()[start..].windows(needle.len()).position(|part| part == needle.units()).map(|n| n + start)
    };
    Ok(JsValue::num(found.map_or(-1.0, |n| n as f64)))
}

fn native_string_is_well_formed(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(JsValue::Bool(string_this_units(i, this)?.is_well_formed()))
}
fn native_string_to_well_formed(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(JsValue::str(string_this_units(i, this)?.to_well_formed()))
}
fn native_string_locale_compare(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    intl_string_locale_compare(i, this, args)
}
fn native_string_normalize(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let source = string_this_units(i, this)?;
    let form = match args.first() { None | Some(JsValue::Undefined) => "NFC".to_string(), Some(v) => i.to_string(v)? };
    if !matches!(form.as_str(), "NFC" | "NFD" | "NFKC" | "NFKD") {
        let constructor = i.global_env.resolve("RangeError")
            .ok_or_else(|| Throw::range_error("invalid normalization form"))?;
        let error = i.construct(&constructor, vec![JsValue::str("invalid normalization form")])?;
        return Err(Throw(error));
    }
    // Deliberately bounded table for common Latin compositions. Other units
    // remain untouched; a complete Unicode table belongs to a later ticket.
    const PAIRS: &[(u16, u16, u16)] = &[
        (0x00c0, 0x0041, 0x0300), (0x00c1, 0x0041, 0x0301),
        (0x00c4, 0x0041, 0x0308), (0x00c5, 0x0041, 0x030a),
        (0x00c7, 0x0043, 0x0327), (0x00c8, 0x0045, 0x0300),
        (0x00c9, 0x0045, 0x0301), (0x00d1, 0x004e, 0x0303),
        (0x00d6, 0x004f, 0x0308), (0x00dc, 0x0055, 0x0308),
        (0x00e0, 0x0061, 0x0300), (0x00e1, 0x0061, 0x0301),
        (0x00e4, 0x0061, 0x0308), (0x00e5, 0x0061, 0x030a),
        (0x00e7, 0x0063, 0x0327), (0x00e8, 0x0065, 0x0300),
        (0x00e9, 0x0065, 0x0301), (0x00f1, 0x006e, 0x0303),
        (0x00f6, 0x006f, 0x0308), (0x00fc, 0x0075, 0x0308),
        (0x0100, 0x0041, 0x0304),
    ];
    let mut decomposed = Vec::with_capacity(source.len() + 4);
    for &unit in source.units() {
        if let Some(&(_, first, mark)) = PAIRS.iter().find(|(composed, _, _)| *composed == unit) {
            decomposed.extend([first, mark]);
        } else if unit == 0x212b && form.starts_with("NFK") {
            decomposed.extend([0x0041, 0x030a]);
        } else {
            decomposed.push(unit);
        }
    }
    if matches!(form.as_str(), "NFD" | "NFKD") {
        return Ok(JsValue::str_units(decomposed));
    }
    let mut composed = Vec::with_capacity(decomposed.len());
    let mut at = 0;
    while at < decomposed.len() {
        if at + 1 < decomposed.len() {
            if let Some(&(unit, _, _)) = PAIRS.iter().find(|(_, first, mark)|
                *first == decomposed[at] && *mark == decomposed[at + 1]) {
                composed.push(unit);
                at += 2;
                continue;
            }
        }
        composed.push(decomposed[at]);
        at += 1;
    }
    Ok(JsValue::str_units(composed))
}

fn native_string_includes(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let source = string_this_units(i, this)?;
    let search_value = args.first().cloned().unwrap_or(JsValue::Undefined);
    if std_is_regexp(i, &search_value)? {
        return Err(Throw::type_error("searchString must not be a RegExp"));
    }
    let needle = i.to_js_string(&search_value)?;
    let len = source.len();
    let position = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    let start = to_integer_or_infinity(i, &position)?.clamp(0.0, len as f64) as usize;
    if needle.is_empty() {
        return Ok(JsValue::Bool(true));
    }
    if needle.len() > len - start {
        return Ok(JsValue::Bool(false));
    }
    Ok(JsValue::Bool(source.units()[start..].windows(needle.len()).any(|part| part == needle.units())))
}

fn native_string_replace(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let source = string_this_units(i, this)?;
    let search_value = args.first().cloned().unwrap_or(JsValue::Undefined);
    if search_value.is_object() {
        let replacer = i.get_prop(&search_value, WellKnownSymbol::Replace.symbol())?;
        if !replacer.is_null() && !replacer.is_undefined() {
            if callable_function(&replacer).is_none() {
                return Err(Throw::type_error("@@replace is not a function"));
            }
            let replace_value = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            return i.call_value(&replacer, search_value, vec![JsValue::str(source), replace_value]);
        }
    }
    let search = i.to_js_string(&search_value)?;
    let replacement = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    // Step 6: a non-callable replaceValue is ToString-converted here,
    // unconditionally and before the search (step 7), even when the search
    // string is never found -- observable through a `toString` side effect.
    let replacement = if callable_function(&replacement).is_some() {
        replacement
    } else {
        JsValue::str(i.to_js_string(&replacement)?)
    };
    let position = if search.is_empty() { Some(0) } else {
        source.units().windows(search.len()).position(|part| part == search.units())
    };
    if let Some(position) = position {
        let replaced = std_string_replacement(i, &replacement, &source, &search, position)?;
        let mut output = Vec::with_capacity(source.len() + replaced.len());
        output.extend_from_slice(&source.units()[..position]);
        output.extend_from_slice(&replaced);
        output.extend_from_slice(&source.units()[position + search.len()..]);
        Ok(JsValue::str_units(output))
    } else {
        Ok(JsValue::str(source))
    }
}

fn native_string_starts_with(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let source = string_this_units(i, this)?;
    let search_value = args.first().cloned().unwrap_or(JsValue::Undefined);
    if std_is_regexp(i, &search_value)? {
        return Err(Throw::type_error("searchString must not be a RegExp"));
    }
    let search = i.to_js_string(&search_value)?;
    let len = source.len();
    let position = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    let start = to_integer_or_infinity(i, &position)?.clamp(0.0, len as f64) as usize;
    if search.is_empty() {
        return Ok(JsValue::Bool(true));
    }
    let end = start + search.len();
    Ok(JsValue::Bool(end <= len && source.units()[start..end] == *search.units()))
}

fn native_string_ends_with(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let source = string_this_units(i, this)?;
    let search_value = args.first().cloned().unwrap_or(JsValue::Undefined);
    if std_is_regexp(i, &search_value)? {
        return Err(Throw::type_error("searchString must not be a RegExp"));
    }
    let search = i.to_js_string(&search_value)?;
    let len = source.len();
    let pos = match args.get(1) {
        Some(value) if !value.is_undefined() => to_integer_or_infinity(i, value)?,
        _ => len as f64,
    };
    let end = pos.clamp(0.0, len as f64) as usize;
    if search.is_empty() {
        return Ok(JsValue::Bool(true));
    }
    let Some(start) = end.checked_sub(search.len()) else {
        return Ok(JsValue::Bool(false));
    };
    Ok(JsValue::Bool(source.units()[start..end] == *search.units()))
}

fn native_array_reduce(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let object = array_receiver(i, this)?;
    let length = array_like_length(i, &object)?;
    let callback = args
        .first()
        .ok_or_else(|| Throw::type_error("array reducer is not a function"))?;
    if callable_function(callback).is_none() {
        return Err(Throw::type_error("array reducer is not a function"));
    }
    let mut index = 0usize;
    let mut accumulator = args.get(1).cloned();
    while index < length {
        if !array_has(i, &object, index)? { index += 1; continue; }
        let value = i.get_prop(&object, &index.to_string())?;
        accumulator = Some(if let Some(current) = accumulator {
            i.call_value(callback, JsValue::Undefined,
                vec![current, value, JsValue::num(index as f64), object.clone()])?
        } else { value });
        index += 1;
    }
    accumulator.ok_or_else(|| Throw::type_error("reduce of empty array with no initial value"))
}

fn native_array_includes(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let object = array_receiver(i, this)?;
    let length = array_search_length(i, &object)?;
    if length == 0 { return Ok(JsValue::Bool(false)); }
    let search = args.first().cloned().unwrap_or(JsValue::Undefined);
    let start = array_index(i, args.get(1), length, 0.0)?;
    for index in start..length {
        let value = array_get_index(i, &object, index)?;
        if value == search || matches!((&value, &search), (JsValue::Num(a), JsValue::Num(b)) if a.is_nan() && b.is_nan()) {
            return Ok(JsValue::Bool(true));
        }
    }
    Ok(JsValue::Bool(false))
}

fn native_array_index_of(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let object = array_receiver(i, this)?;
    let length = array_search_length(i, &object)?;
    if length == 0 { return Ok(JsValue::num(-1.0)); }
    let search = args.first().cloned().unwrap_or(JsValue::Undefined);
    let start = array_index(i, args.get(1), length, 0.0)?;
    for index in start..length {
        if array_has(i, &object, index)? && array_get_index(i, &object, index)? == search {
            return Ok(JsValue::num(index as f64));
        }
    }
    Ok(JsValue::num(-1.0))
}
fn native_boolean(_: &mut Interpreter, this: JsValue, a: Vec<JsValue>) -> Result<JsValue, Throw> {
    let value = JsValue::Bool(a.first().map(|x| x.to_boolean()).unwrap_or(false));
    if let JsValue::Object(object) = &this {
        object.borrow_mut().boxed_primitive = Some(value);
        Ok(this)
    } else {
        Ok(value)
    }
}
fn native_is_nan(i: &mut Interpreter, _: JsValue, a: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(JsValue::Bool(
        i.to_number(a.first().unwrap_or(&JsValue::Undefined))?
            .is_nan(),
    ))
}
fn native_log(i: &mut Interpreter, _: JsValue, a: Vec<JsValue>) -> Result<JsValue, Throw> {
    let mut s = String::new();
    for (n, x) in a.iter().enumerate() {
        if n > 0 {
            s.push(' ')
        }
        s.push_str(&i.to_string(x)?);
    }
    i.output.borrow_mut().push(s);
    Ok(JsValue::Undefined)
}

fn native_set_timeout(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    if callable_function(&callback).is_none() {
        return Err(Throw::type_error("setTimeout callback is not callable"));
    }
    // WebIDL `long`: ToNumber, truncate, modulo 2^32, then signed wrap.
    // The DOM event loop applies nonnegative/nesting timer clamping.
    let delay = i.to_int32(args.get(1).unwrap_or(&JsValue::num(0.0)))?;
    let id = i.schedule_timer(callback, delay, args.into_iter().skip(2).collect())?;
    Ok(JsValue::num(id as f64))
}

fn native_clear_timeout(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let id = i
        .to_number(args.first().unwrap_or(&JsValue::Undefined))?
        .max(0.0) as u32;
    i.cancel_timer(id)?;
    Ok(JsValue::Undefined)
}

fn native_set_interval(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    if callable_function(&callback).is_none() {
        return Err(Throw::type_error("setInterval callback is not callable"));
    }
    let delay = i.to_int32(args.get(1).unwrap_or(&JsValue::num(0.0)))?;
    let id = i.schedule_interval(callback, delay, args.into_iter().skip(2).collect())?;
    Ok(JsValue::num(id as f64))
}

fn native_request_animation_frame(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    if callable_function(&callback).is_none() {
        return Err(Throw::type_error(
            "requestAnimationFrame callback is not callable",
        ));
    }
    let id = i.schedule_animation_frame(callback)?;
    Ok(JsValue::num(id as f64))
}

fn native_cancel_animation_frame(
    i: &mut Interpreter, _: JsValue, args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let id = i.to_number(args.first().unwrap_or(&JsValue::Undefined))?.max(0.0) as u32;
    i.cancel_animation_frame(id)?;
    Ok(JsValue::Undefined)
}

fn native_queue_microtask(
    i: &mut Interpreter, _: JsValue, args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    i.queue_microtask(args.first().cloned().unwrap_or(JsValue::Undefined))?;
    Ok(JsValue::Undefined)
}

// testharness.js distinguishes callback setup from property-object setup with
// instanceof Function, and checks setup properties with hasOwnProperty.
//
// Shared by %Function%, %AsyncFunction%, %GeneratorFunction% and
// %AsyncGeneratorFunction% (CreateDynamicFunction): `prefix` selects the
// kind by spelling out the keyword(s) that precede the synthesized
// function's name, so the parser sets `is_async`/`is_generator` on the
// resulting `ast::FunctionItem` exactly as it would for a literal
// declaration -- `make_function` then places the result on the matching
// shared `[[Prototype]]` (async_function_proto, generator_function_proto,
// ...) with no further kind-specific logic needed here. Call and construct
// behave identically (this ignores `this`/`new.target`; a subclassed
// construction's `[[Prototype]]` is fixed up generically afterward by
// `construct_with_new_target`).
fn native_dynamic_function(i: &mut Interpreter, args: Vec<JsValue>, prefix: &str) -> Result<JsValue, Throw> {
    let mut parts = args
        .iter()
        .map(|a| i.to_string(a))
        .collect::<Result<Vec<_>, _>>()?;
    let body = parts.pop().unwrap_or_default();
    let source = format!("{prefix} anonymous({}) {{\n{}\n}}", parts.join(","), body);
    let script = crate::parse_script(&source).map_err(|e| Throw::syntax_error(e.to_string()))?;
    if script.body.len() != 1 {
        return Err(Throw::syntax_error("invalid Function constructor source"));
    }
    if let ast::StmtKind::Function(function) = &script.body[0].kind {
        let previous_strict = i.strict_eval_context;
        i.strict_eval_context = false;
        let value = i.make_function(function, &i.global_with_active_source_context());
        i.strict_eval_context = previous_strict;
        Ok(value)
    } else {
        Err(Throw::syntax_error("invalid Function constructor source"))
    }
}

fn native_function_constructor(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    native_dynamic_function(i, args, "function")
}

fn native_async_function_constructor(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    native_dynamic_function(i, args, "async function")
}

fn native_generator_function_constructor(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    native_dynamic_function(i, args, "function*")
}

fn native_async_generator_function_constructor(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    native_dynamic_function(i, args, "async function*")
}
fn native_has_own_property(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let key = i.to_property_key(args.first().unwrap_or(&JsValue::Undefined))?;
    let JsValue::Object(object) = std_legacy_to_object(i, this)? else { unreachable!() };
    Ok(JsValue::Bool(i.default_get_own_property(&object, &key)?.is_some()))
}

fn native_property_is_enumerable(
    i: &mut Interpreter, this: JsValue, args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let key = i.to_property_key(args.first().unwrap_or(&JsValue::Undefined))?;
    let JsValue::Object(object) = std_legacy_to_object(i, this)? else { unreachable!() };
    Ok(JsValue::Bool(i.default_get_own_property(&object, &key)?.is_some_and(|property| property.enumerable)))
}

#[cfg(test)]
mod promise_constructor_allocation_tests {
    use super::*;
    fn check(source: &str) {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i=Interpreter::new();
            assert_eq!(i.run_script_with_backend(source,backend).unwrap(),JsValue::Bool(true),"{backend:?}; {:?}",i.output.borrow());
        }
    }
    #[test]
    fn executor_validation_precedes_prototype_get_and_getter_is_called_once() {
        check(r#"
          var valid=true,count=0,called=0,error={},proto={};
          var NT=new Proxy(function(){},{get(t,k,r){if(k==='prototype'){count++;throw error;}return Reflect.get(t,k,r);}});
          for(var executor of [undefined,null,1,'x',{},Symbol(),2n]){
            try{Reflect.construct(Promise,[executor],NT);valid=false;}catch(e){valid=valid && e instanceof TypeError && count===0;}
          }
          try{Reflect.construct(Promise,[()=>called++],NT);valid=false;}catch(e){valid=valid && e===error && count===1 && called===0;}
          count=0;NT=new Proxy(function(){},{get(t,k,r){if(k==='prototype'){count++;return proto;}return Reflect.get(t,k,r);}});
          var resolve,p=Reflect.construct(Promise,[r=>{called++;resolve=r;}],NT);
          valid=valid && count===1 && called===1 && Object.getPrototypeOf(p)===proto;
          valid=valid && typeof resolve==='function';resolve(7);
          var fake={};fake.prototype=proto;try{Reflect.construct(Promise,[()=>{}],fake);valid=false;}catch(e){valid=valid && e instanceof TypeError;}
          valid;
        "#);
    }
    #[test]
    fn callable_revocation_and_prototype_revocation_have_distinct_boundaries() {
        check(r#"
          var valid=true,count=0,executed=0,proto={};
          var r=Proxy.revocable(function(){},{});r.revoke();
          var nt=new Proxy(function(){},{get(t,k,r){if(k==='prototype'){count++;return proto;}return Reflect.get(t,k,r);}});
          var p=Reflect.construct(Promise,[r.proxy],nt);
          valid=valid && count===1 && Object.getPrototypeOf(p)===proto;
          var revokeTarget=Proxy.revocable(function(){},{get(t,k,r){if(k==='prototype'){revokeTarget.revoke();return proto;}return Reflect.get(t,k,r);}});
          var q=Reflect.construct(Promise,[()=>executed++],revokeTarget.proxy);
          valid=valid && Object.getPrototypeOf(q)===proto && executed===1;
          var revokePrimitive=Proxy.revocable(function(){},{get(t,k,r){if(k==='prototype'){revokePrimitive.revoke();return null;}return Reflect.get(t,k,r);}});
          try{Reflect.construct(Promise,[()=>executed++],revokePrimitive.proxy);valid=false;}catch(e){valid=valid && e instanceof TypeError && executed===1;}
          for(var executor of [undefined,()=>{}]){try{Reflect.construct(Promise,[executor],r.proxy);valid=false;}catch(e){valid=valid && e instanceof TypeError;}}
          valid;
        "#);
    }
    #[test]
    fn subclass_bound_proxy_and_saved_intrinsic_allocations_preserve_native_semantics() {
        check(r#"
          var valid=true,original=Promise;
          class Sub extends Promise {constructor(e){super(e);this.marker=1;}}
          var s=new Sub(r=>r(3));valid=valid && s instanceof Sub && s instanceof Promise && s.marker===1;
          var bound=Promise.bind(null),b=new bound(()=>{});valid=valid && Object.getPrototypeOf(b)===Promise.prototype;
          var proxy=new Proxy(Promise,{}),p=new proxy(()=>{});valid=valid && Object.getPrototypeOf(p)===Promise.prototype;
          var NT=function(){};NT.prototype=null;var q=Reflect.construct(Promise,[()=>{}],NT);
          valid=valid && Object.getPrototypeOf(q)===Promise.prototype;
          Promise=function(){throw 'replacement';};var saved=new original(()=>{});valid=valid && Object.getPrototypeOf(saved)===original.prototype;
          for(var fn of [original,bound,proxy]){try{fn(()=>{});valid=false;}catch(e){valid=valid && e instanceof TypeError;}}
          var fake=function Promise(){};fake.prototype={fake:true};var ordinary=new fake;
          valid=valid && ordinary.fake===true && Object.getPrototypeOf(ordinary)===fake.prototype;valid;
        "#);
    }
    #[test]
    fn executor_errors_reject_and_post_resolve_throws_preserve_original_identity() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i=Interpreter::new();
            i.run_script_with_backend(r#"
              var error={},resolve,reject;
              new Promise(()=>{throw error;}).catch(e=>console.log('identity',e===error));
              new Promise(r=>{r(7);throw error;}).then(v=>console.log('fulfilled',v));
              new Promise((r,j)=>{j(error);r(8);throw 9;}).catch(e=>console.log('first',e===error));
              var revoked=Proxy.revocable(function(){},{});revoked.revoke();
              new Promise(revoked.proxy).catch(e=>console.log('revoked',e instanceof TypeError));
              var p=new Promise((r,j)=>{resolve=r;reject=j;});resolve(11);reject(error);
              p.then(v=>console.log('escaped',v));
              console.log('metadata',resolve.name==='',reject.name==='',resolve.length,reject.length,!Object.hasOwn(resolve,'prototype'));
            "#,backend).unwrap();
            let mut out=i.output.borrow().clone();out.sort();
            assert_eq!(out,["escaped 11","first true","fulfilled 7","identity true","metadata true true 1 1 true","revoked true"],"{backend:?}");
        }
    }
    #[test]
    fn fallback_uses_new_target_realm_and_errors_use_actual_callee_realm() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i=Interpreter::new();let child=i.binding_create_realm(i.binding_create_object(None).unwrap()).unwrap();
            for(name,source) in [("foreignPromise","Promise"),("foreignProto","Promise.prototype"),("foreignTypeError","TypeError.prototype"),
                ("foreignNT","(()=>{var NT=function(){};NT.prototype=null;return NT;})()"),("foreignBoundNT","(function(){}).bind(null)")] {
                let value=i.binding_eval_in_realm(child,source).unwrap();i.global_env.declare_var(name,value).unwrap();
            }
            assert_eq!(i.run_script_with_backend(r#"
              var valid=true,NT=function(){};NT.prototype=null;
              var a=Reflect.construct(foreignPromise,[()=>{}],NT);valid=valid && Object.getPrototypeOf(a)===Promise.prototype;
              for(var nt of [foreignNT,foreignBoundNT,new Proxy(foreignNT,{})]){
                var b=Reflect.construct(Promise,[()=>{}],nt);valid=valid && Object.getPrototypeOf(b)===foreignProto;
              }
              var count=0,trap=new Proxy(foreignNT,{get(t,k,r){if(k==='prototype'){count++;return null;}return Reflect.get(t,k,r);}});
              var c=Reflect.construct(Promise,[()=>{}],trap);valid=valid && Object.getPrototypeOf(c)===foreignProto && count===1;
              try{Reflect.construct(foreignPromise,[null],trap);valid=false;}catch(e){valid=valid && Object.getPrototypeOf(e)===foreignTypeError && count===1;}
              var own={},objectNT=new Proxy(foreignNT,{get(t,k,r){if(k==='prototype')return own;return Reflect.get(t,k,r);}});
              valid=valid && Object.getPrototypeOf(Reflect.construct(foreignPromise,[()=>{}],objectNT))===own;
              valid;
            "#,backend).unwrap(),JsValue::Bool(true),"{backend:?}");
        }
    }
}
