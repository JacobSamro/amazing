fn native_object(i: &mut Interpreter, _: JsValue, a: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(match a.into_iter().next() {
        Some(value @ JsValue::Object(_)) => value,
        Some(JsValue::Num(number)) => {
            let mut object = JsObject::ordinary();
            object.proto = Some(i.object_proto.clone());
            object.set_own("[[NumberData]]", JsValue::Num(number));
            JsValue::object(object)
        }
        Some(JsValue::Bool(boolean)) => {
            let mut object = JsObject::ordinary();
            object.proto = Some(i.object_proto.clone());
            object.set_own("[[BooleanData]]", JsValue::Bool(boolean));
            JsValue::object(object)
        }
        Some(JsValue::Str(string)) => {
            let mut object = JsObject::ordinary();
            object.proto = Some(i.string_proto.clone());
            object.set_own("[[StringData]]", JsValue::Str(string));
            JsValue::object(object)
        }
        Some(JsValue::BigInt(value)) => {
            let mut object = JsObject::ordinary();
            object.proto = i.global_env.resolve("BigInt")
                .and_then(|constructor| i.get_prop(&constructor, "prototype").ok())
                .and_then(|prototype| if let JsValue::Object(object) = prototype { Some(object) } else { None });
            object.set_own("[[BigIntData]]", JsValue::BigInt(value));
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
    _: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let JsValue::Object(window) = this else {
        return Err(Throw::type_error(
            "location getter receiver is not an object",
        ));
    };
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
    let mut node = dom_id(&this)?;
    let selector = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    loop {
        let matches = dom_host(i)?.borrow().matches_selector(node, &selector);
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
    let prototype = args.first().cloned().unwrap_or(JsValue::Null);
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
        if !properties.is_null() && !properties.is_undefined() {
            let _ = native_object_assign(
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
    match args.first() {
        Some(JsValue::Symbol(_)) => Ok(JsValue::Object(i.symbol_proto.clone())),
        Some(JsValue::Object(object)) => Ok(object
            .borrow()
            .proto
            .clone()
            .map(JsValue::Object)
            .unwrap_or(JsValue::Null)),
        _ => Err(Throw::type_error("getPrototypeOf target is not an object")),
    }
}

fn native_object_set_prototype_of(
    _: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let object = match args.first() {
        Some(JsValue::Object(object)) => object.clone(),
        _ => return Err(Throw::type_error("setPrototypeOf target is not an object")),
    };
    let prototype = match args.get(1).cloned().unwrap_or(JsValue::Null) {
        JsValue::Null => None,
        JsValue::Object(prototype) => Some(prototype),
        _ => return Err(Throw::type_error("prototype must be an object or null")),
    };
    object.borrow_mut().proto = prototype;
    Ok(JsValue::Object(object))
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
    if let Some(own) = binary_index_descriptor(object, &key) { return Ok(JsValue::Bool(own.is_some())); }
    let own = match key.as_string() {
        Some(name) => i.legacy_own_property(object, name, false)?.is_some(),
        None => object.borrow().own_prop(&key).is_some(),
    };
    Ok(JsValue::Bool(own))
}

fn native_array_is_array(
    _: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    Ok(JsValue::Bool(
        matches!(args.first(), Some(JsValue::Object(object)) if matches!(object.borrow().exotic, Exotic::Array)),
    ))
}

fn native_array_from(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let source = args.first().cloned().unwrap_or(JsValue::Undefined);
    if matches!(source, JsValue::Undefined | JsValue::Null) { return Err(Throw::type_error("Array.from requires an object")); }
    if let Some(callback) = args.get(1).filter(|callback| !matches!(callback, JsValue::Undefined)) {
        if callable_function(callback).is_none() {
            return Err(Throw::type_error("Array.from mapFn is not callable"));
        }
    }
    let iterator = i.get_prop(&source, WellKnownSymbol::Iterator.symbol())?;
    let mut values = if matches!(iterator, JsValue::Undefined | JsValue::Null) {
        let length = i.get_prop(&source, "length")?;
        let number = i.to_number(&length)?;
        let length = if number.is_nan() || number <= 0.0 { 0 }
            else if !number.is_finite() || number > 1_000_000.0 { return Err(Throw::range_error("Array.from source too large")); }
            else { number.trunc() as usize };
        (0..length).map(|index| i.get_prop(&source, index.to_string())).collect::<Result<Vec<_>, _>>()?
    } else if callable_function(&iterator).is_some() { i.iterate(&source)? }
    else { return Err(Throw::type_error("Array.from iterator is not callable")); };
    if let Some(callback) = args.get(1).filter(|callback| !matches!(callback, JsValue::Undefined)) {
        for (index, value) in values.iter_mut().enumerate() {
            *value = i.call_value(
                callback,
                args.get(2).cloned().unwrap_or(JsValue::Undefined),
                vec![value.clone(), JsValue::num(index as f64)],
            )?;
        }
    }
    array_values(i, values)
}

fn native_array_splice(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let JsValue::Object(object) = &this else { return Err(Throw::type_error("splice receiver is not an object")); };
    let len_value = i.get_prop(&this, "length")?;
    let len_number = i.to_number(&len_value)?;
    let len = if len_number.is_nan() || len_number <= 0.0 { 0usize }
        else if len_number > 1_000_000.0 || !len_number.is_finite() { return Err(Throw::range_error("splice array too large")); }
        else { len_number.trunc() as usize };
    let start_number = i.to_number(args.first().unwrap_or(&JsValue::Undefined))?;
    let start_integer = if start_number.is_nan() { 0i64 }
        else if start_number.is_infinite() { if start_number.is_sign_negative() { i64::MIN } else { i64::MAX } }
        else { start_number.trunc() as i64 };
    let start = if start_integer < 0 { (len as i64).saturating_add(start_integer).max(0) as usize }
        else { (start_integer as usize).min(len) };
    let delete_count = if args.is_empty() { 0 }
        else if args.len() == 1 { len - start }
        else {
            let count = i.to_number(&args[1])?;
            if count.is_nan() || count <= 0.0 { 0 }
            else if !count.is_finite() { len - start }
            else { (count.trunc() as usize).min(len - start) }
        };
    let inserted = args.get(2..).unwrap_or(&[]);
    let new_len = len - delete_count + inserted.len();
    if new_len > 1_000_000 { return Err(Throw::range_error("splice result too large")); }
    let mut values = Vec::with_capacity(len);
    for index in 0..len {
        let key = index.to_string();
        values.push(if object.borrow().has_property(&key) { Some(i.get_prop(&this, &key)?) } else { None });
    }
    let deleted = {
        let mut raw = JsObject::array(); raw.proto = Some(i.array_proto.clone());
        let array = JsValue::object(raw);
        for (index, value) in values[start..start+delete_count].iter().enumerate() {
            if let Some(value) = value { i.set_prop(&array, index.to_string(), value.clone())?; }
        }
        i.set_prop(&array, "length", JsValue::num(delete_count as f64))?;
        array
    };
    values.splice(start..start+delete_count, inserted.iter().cloned().map(Some));
    for (index, value) in values.into_iter().enumerate() {
        let key = index.to_string();
        if let Some(value) = value { i.set_prop(&this, &key, value)?; }
        else if !object.borrow_mut().delete_own(&key) { return Err(Throw::type_error("cannot delete array element")); }
    }
    for index in new_len..len {
        if !object.borrow_mut().delete_own(index.to_string()) { return Err(Throw::type_error("cannot delete array element")); }
    }
    i.set_prop(&this, "length", JsValue::num(new_len as f64))?;
    Ok(deleted)
}

fn native_array_of(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    array_values(i, args)
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
        Some(value) => i.iterate(value)?,
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
    let data = Obj(Rc::new(RefCell::new(JsObject::ordinary())));
    data.borrow_mut().set_own("target", this);
    data.borrow_mut()
        .set_own("this", args.first().cloned().unwrap_or(JsValue::Undefined));
    data.borrow_mut()
        .set_own("args", array_values(i, args.into_iter().skip(1).collect())?);
    let bound = Interpreter::native("bound", native_bound, &i.function_proto);
    if let JsValue::Object(function) = &bound {
        function.borrow_mut().func.as_mut().unwrap().home = Some(data);
    }
    Ok(bound)
}

fn native_bound(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = native_data(i)?;
    let target = data
        .borrow()
        .get_own("target")
        .ok_or_else(|| Throw::type_error("invalid bound function"))?;
    let receiver = data.borrow().get_own("this").unwrap_or(JsValue::Undefined);
    let bound_args = data
        .borrow()
        .get_own("args")
        .map(|value| i.iterate(&value))
        .transpose()?
        .unwrap_or_default();
    i.call_value(
        &target,
        receiver,
        bound_args.into_iter().chain(args).collect(),
    )
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
    let descriptor = match args.get(2) {
        Some(JsValue::Object(object)) => object.clone(),
        _ => return Err(Throw::type_error("property descriptor is not an object")),
    };
    let key = i.to_property_key(args.get(1).unwrap_or(&JsValue::Undefined))?;
    if let Some(result) = binary_define_index_object(i, &target, &key, &descriptor) {
        // DefinePropertyOrThrow: a rejected integer-indexed definition throws here.
        if !result? { return Err(Throw::type_error("could not define TypedArray index")); }
        return Ok(JsValue::Object(target));
    }
    let has = |name: &str| descriptor.borrow().has_property(name);
    let accessor = has("get") || has("set");
    let data = has("value") || has("writable");
    if accessor && data {
        return Err(Throw::type_error(
            "descriptor mixes data and accessor fields",
        ));
    }
    if target.borrow().own_prop(&key).is_none() && !target.borrow().extensible {
        return Err(Throw::type_error("object is not extensible"));
    }
    let value = if has("value") {
        i.get_prop(&JsValue::Object(descriptor.clone()), "value")?
    } else {
        JsValue::Undefined
    };
    let writable = if has("writable") {
        i.get_prop(&JsValue::Object(descriptor.clone()), "writable")?
            .to_boolean()
    } else {
        false
    };
    let enumerable = if has("enumerable") {
        i.get_prop(&JsValue::Object(descriptor.clone()), "enumerable")?
            .to_boolean()
    } else {
        false
    };
    let configurable = if has("configurable") {
        i.get_prop(&JsValue::Object(descriptor.clone()), "configurable")?
            .to_boolean()
    } else {
        false
    };
    let getter = if has("get") {
        match i.get_prop(&JsValue::Object(descriptor.clone()), "get")? {
            JsValue::Undefined => None,
            value => Some(
                callable_function(&value)
                    .map(Box::new)
                    .ok_or_else(|| Throw::type_error("property getter is not callable"))?,
            ),
        }
    } else {
        None
    };
    let setter = if has("set") {
        match i.get_prop(&JsValue::Object(descriptor.clone()), "set")? {
            JsValue::Undefined => None,
            value => Some(
                callable_function(&value)
                    .map(Box::new)
                    .ok_or_else(|| Throw::type_error("property setter is not callable"))?,
            ),
        }
    } else {
        None
    };
    let property = Property {
        value,
        writable: if accessor { false } else { writable },
        enumerable,
        configurable,
        getter,
        setter,
    };
    if let Some(result) = binary_define_index(i, &target, &key, &property) { result?; return Ok(JsValue::Object(target)); }
    if let Some(name) = key.as_string() {
        if i.legacy_define(&target, name, &property)? { return Ok(JsValue::Object(target)); }
    }
    target
        .borrow_mut()
        .define(&key, property)
        .map_err(|_| Throw::type_error("could not define property"))?;
    Ok(JsValue::Object(target))
}

fn native_prevent_extensions(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let target = args.first().cloned().unwrap_or(this);
    match target {
        JsValue::Object(object) => {
            if i.legacy_for(&object).is_some() { return Err(Throw::type_error("cannot prevent extensions on a legacy platform object")); }
            object.borrow_mut().extensible = false;
            Ok(JsValue::Object(object))
        }
        _ => Err(Throw::type_error(
            "preventExtensions target is not an object",
        )),
    }
}

fn native_is_extensible(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    match args.first() {
        Some(JsValue::Object(object)) => Ok(JsValue::Bool(i.legacy_for(object).is_some() || object.borrow().extensible)),
        _ => Err(Throw::type_error("isExtensible target is not an object")),
    }
}

fn native_date(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    if let JsValue::Object(object) = &this {
        let value = i.to_number(args.first().unwrap_or(&JsValue::num(0.0)))?;
        object
            .borrow_mut()
            .set_own("[[DateValue]]", JsValue::Num(value));
        Ok(this)
    } else {
        Ok(JsValue::str(""))
    }
}

fn native_date_to_string(
    _: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    if let JsValue::Object(object) = this {
        if object.borrow().get_own("[[DateValue]]").is_some() {
            return Ok(JsValue::str(
                "Thu Jan 01 1970 00:00:00 GMT+0000 (Coordinated Universal Time)",
            ));
        }
    }
    Ok(JsValue::str("Invalid Date"))
}

fn native_eval(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let source = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    let program =
        crate::parse_script(&source).map_err(|error| Throw::syntax_error(error.to_string()))?;
    i.run_program(&program)
}

fn native_error_value(i: &mut Interpreter, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let message = match args.first() {
        Some(JsValue::Undefined) | None => None,
        Some(value) => Some(i.to_string(value)?),
    };
    let object = Obj(Rc::new(RefCell::new({
        let mut object = JsObject::ordinary();
        object.exotic = Exotic::Error;
        if let Some(message) = message {
            object.set_own("message", JsValue::str(message));
        }
        if let Some(function) = i.native_stack.last() {
            if let Some(JsValue::Object(proto)) = function.borrow().get_own("prototype") {
                object.proto = Some(proto);
            }
        }
        object
    })));
    Ok(JsValue::Object(object))
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
    let value = Interpreter::native(name, f, &i.function_proto);
    if let JsValue::Object(o) = &value {
        o.borrow_mut()
            .set_own("[[PromiseTarget]]", JsValue::Object(target.clone()));
    }
    value
}

fn native_promise(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let executor = args.first().cloned().unwrap_or(JsValue::Undefined);
    if callable_function(&executor).is_none() {
        return Err(Throw::type_error("Promise resolver is not a function"));
    }
    let promise = promise_object(&new_promise(i, PromiseFate::Pending))?;
    let resolve = promise_callback(i, "resolve", &promise, native_promise_resolve);
    let reject = promise_callback(i, "reject", &promise, native_promise_reject);
    if let Err(Throw(value)) = i.call_value(&executor, JsValue::Undefined, vec![resolve, reject]) {
        settle_promise(&promise, PromiseFate::Rejected(value), i)?;
    }
    Ok(JsValue::Object(promise))
}

fn current_promise_target(i: &Interpreter) -> Option<Obj> {
    let function = i.native_stack.last()?;
    match function.borrow().get_own("[[PromiseTarget]]") {
        Some(JsValue::Object(target)) => Some(target),
        _ => None,
    }
}

fn native_promise_resolve(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    if let Some(target) = current_promise_target(i) {
        settle_promise(
            &target,
            PromiseFate::Fulfilled(args.first().cloned().unwrap_or(JsValue::Undefined)),
            i,
        )?;
        return Ok(JsValue::Undefined);
    }
    let value = args.first().cloned().unwrap_or(JsValue::Undefined);
    if matches!(value, JsValue::Object(ref o) if matches!(o.borrow().exotic, Exotic::Promise(_))) {
        Ok(value)
    } else {
        Ok(new_promise(i, PromiseFate::Fulfilled(value)))
    }
}

fn native_promise_reject(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    if let Some(target) = current_promise_target(i) {
        settle_promise(
            &target,
            PromiseFate::Rejected(args.first().cloned().unwrap_or(JsValue::Undefined)),
            i,
        )?;
        return Ok(JsValue::Undefined);
    }
    Ok(new_promise(
        i,
        PromiseFate::Rejected(args.first().cloned().unwrap_or(JsValue::Undefined)),
    ))
}

fn settle_promise(target: &Obj, fate: PromiseFate, i: &mut Interpreter) -> Result<(), Throw> {
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
    let result = match (reaction.handler, fate) {
        (Some(handler), PromiseFate::Fulfilled(value)) => {
            let function = JsValue::Object(Obj(Rc::new(RefCell::new(JsObject::function(handler)))));
            match i.call_value(&function, JsValue::Undefined, vec![value]) {
                Ok(value) => PromiseFate::Fulfilled(value),
                Err(Throw(value)) => PromiseFate::Rejected(value),
            }
        }
        (Some(handler), PromiseFate::Rejected(value)) if reaction.is_reject_handler => {
            let function = JsValue::Object(Obj(Rc::new(RefCell::new(JsObject::function(handler)))));
            match i.call_value(&function, JsValue::Undefined, vec![value]) {
                Ok(value) => PromiseFate::Fulfilled(value),
                Err(Throw(value)) => PromiseFate::Rejected(value),
            }
        }
        (Some(_), PromiseFate::Rejected(value)) => PromiseFate::Rejected(value),
        (None, PromiseFate::Fulfilled(value)) => PromiseFate::Fulfilled(value),
        (None, PromiseFate::Rejected(value)) => PromiseFate::Rejected(value),
        (_, PromiseFate::Pending) => PromiseFate::Pending,
    };
    if let PromiseFate::Fulfilled(value) = &result {
        if adopt_promise_result(&reaction.capability, value, i)? {
            return Ok(());
        }
    }
    settle_promise(&reaction.capability, result, i)
}

fn adopt_promise_result(capability: &Obj, value: &JsValue, i: &mut Interpreter) -> Result<bool, Throw> {
    let Ok(source) = promise_object(value) else {
        return Ok(false);
    };
    if Rc::ptr_eq(&source.0, &capability.0) {
        settle_promise(
            capability,
            PromiseFate::Rejected(JsValue::str("promise resolution cycle")),
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
                    capability: capability.clone(),
                    is_reject_handler: false,
                });
                state.reject_reactions.push(PendingReaction {
                    handler: None,
                    capability: capability.clone(),
                    is_reject_handler: true,
                });
            }
        }
        PromiseFate::Fulfilled(value) => i.enqueue_promise_job(PromiseJob {
            reaction: PendingReaction {
                handler: None,
                capability: capability.clone(),
                is_reject_handler: false,
            },
            fate: PromiseFate::Fulfilled(value),
        })?,
        PromiseFate::Rejected(value) => i.enqueue_promise_job(PromiseJob {
            reaction: PendingReaction {
                handler: None,
                capability: capability.clone(),
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
    let result = promise_object(&new_promise(i, PromiseFate::Pending))?;
    let fulfilled = args.get(0).and_then(callable_function);
    let rejected = args.get(1).and_then(callable_function);
    let state = {
        let mut object = promise.borrow_mut();
        let state = match &mut object.exotic {
            Exotic::Promise(state) => state,
            _ => unreachable!(),
        };
        match &state.state {
            PromiseFate::Pending => {
                state.fulfill_reactions.push(PendingReaction {
                    handler: fulfilled.clone(),
                    capability: result.clone(),
                    is_reject_handler: false,
                });
                state.reject_reactions.push(PendingReaction {
                    handler: rejected.clone(),
                    capability: result.clone(),
                    is_reject_handler: true,
                });
                None
            }
            PromiseFate::Fulfilled(value) => Some(PromiseFate::Fulfilled(value.clone())),
            PromiseFate::Rejected(value) => Some(PromiseFate::Rejected(value.clone())),
        }
    };
    if let Some(fate) = state {
        let reaction = if matches!(fate, PromiseFate::Fulfilled(_)) {
            PendingReaction {
                handler: fulfilled.clone(),
                capability: result.clone(),
                is_reject_handler: false,
            }
        } else {
            PendingReaction {
                handler: rejected.clone(),
                capability: result.clone(),
                is_reject_handler: true,
            }
        };
        i.enqueue_promise_job(PromiseJob { reaction, fate })?;
    }
    Ok(JsValue::Object(result))
}

fn native_promise_catch(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    native_promise_then(
        i,
        this,
        vec![
            JsValue::Undefined,
            args.first().cloned().unwrap_or(JsValue::Undefined),
        ],
    )
}

fn native_promise_finally(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let promise = promise_object(&this)?;
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    let state = match &promise.borrow().exotic {
        Exotic::Promise(state) => state.state.clone(),
        _ => unreachable!(),
    };
    if matches!(state, PromiseFate::Pending) {
        return Ok(this);
    }
    if callable_function(&callback).is_some() {
        i.call_value(&callback, JsValue::Undefined, Vec::new())?;
    }
    Ok(match state {
        PromiseFate::Fulfilled(value) => new_promise(i, PromiseFate::Fulfilled(value)),
        PromiseFate::Rejected(value) => new_promise(i, PromiseFate::Rejected(value)),
        PromiseFate::Pending => unreachable!(),
    })
}

fn native_data(i: &Interpreter) -> Result<Obj, Throw> {
    i.native_stack
        .last()
        .and_then(|function| function.borrow().func.as_ref()?.home.clone())
        .ok_or_else(|| Throw::type_error("missing native Promise callback state"))
}

fn combinator_callback(
    i: &Interpreter,
    name: &str,
    callback: crate::value::NativeFn,
    state: &Obj,
    index: usize,
) -> JsValue {
    let data = Obj(Rc::new(RefCell::new(JsObject::ordinary())));
    data.borrow_mut()
        .set_own("state", JsValue::Object(state.clone()));
    data.borrow_mut()
        .set_own("index", JsValue::num(index as f64));
    let value = Interpreter::native(name, callback, &i.function_proto);
    if let JsValue::Object(function) = &value {
        function.borrow_mut().func.as_mut().unwrap().home = Some(data);
    }
    value
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

fn native_all_fulfill(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let (state, target, index) = combinator_state(i)?;
    let values = state_values(&state, "values")?;
    i.set_prop(
        &values,
        &index.to_string(),
        args.first().cloned().unwrap_or(JsValue::Undefined),
    )?;
    if decrement_remaining(&state)? == 0 {
        settle_promise(&target, PromiseFate::Fulfilled(values), i)?;
    }
    Ok(JsValue::Undefined)
}

fn native_all_reject(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let (_, target, _) = combinator_state(i)?;
    settle_promise(
        &target,
        PromiseFate::Rejected(args.first().cloned().unwrap_or(JsValue::Undefined)),
        i,
    )?;
    Ok(JsValue::Undefined)
}

fn native_race_fulfill(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let (_, target, _) = combinator_state(i)?;
    settle_promise(
        &target,
        PromiseFate::Fulfilled(args.first().cloned().unwrap_or(JsValue::Undefined)),
        i,
    )?;
    Ok(JsValue::Undefined)
}

fn native_race_reject(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let (_, target, _) = combinator_state(i)?;
    settle_promise(
        &target,
        PromiseFate::Rejected(args.first().cloned().unwrap_or(JsValue::Undefined)),
        i,
    )?;
    Ok(JsValue::Undefined)
}

fn settled_record(i: &Interpreter, status: &str, key: &str, value: JsValue) -> JsValue {
    let record = i.object_value();
    if let JsValue::Object(object) = &record {
        object.borrow_mut().set_own("status", JsValue::str(status));
        object.borrow_mut().set_own(key, value);
    }
    record
}

fn native_all_settled_fulfill(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let (state, target, index) = combinator_state(i)?;
    let values = state_values(&state, "values")?;
    i.set_prop(
        &values,
        &index.to_string(),
        settled_record(
            i,
            "fulfilled",
            "value",
            args.first().cloned().unwrap_or(JsValue::Undefined),
        ),
    )?;
    if decrement_remaining(&state)? == 0 {
        settle_promise(&target, PromiseFate::Fulfilled(values), i)?;
    }
    Ok(JsValue::Undefined)
}

fn native_all_settled_reject(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let (state, target, index) = combinator_state(i)?;
    let values = state_values(&state, "values")?;
    i.set_prop(
        &values,
        &index.to_string(),
        settled_record(
            i,
            "rejected",
            "reason",
            args.first().cloned().unwrap_or(JsValue::Undefined),
        ),
    )?;
    if decrement_remaining(&state)? == 0 {
        settle_promise(&target, PromiseFate::Fulfilled(values), i)?;
    }
    Ok(JsValue::Undefined)
}

fn native_any_fulfill(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let (_, target, _) = combinator_state(i)?;
    settle_promise(
        &target,
        PromiseFate::Fulfilled(args.first().cloned().unwrap_or(JsValue::Undefined)),
        i,
    )?;
    Ok(JsValue::Undefined)
}

fn native_any_reject(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let (state, target, index) = combinator_state(i)?;
    let errors = state_values(&state, "errors")?;
    i.set_prop(
        &errors,
        &index.to_string(),
        args.first().cloned().unwrap_or(JsValue::Undefined),
    )?;
    if decrement_remaining(&state)? == 0 {
        settle_promise(&target, PromiseFate::Rejected(errors), i)?;
    }
    Ok(JsValue::Undefined)
}

fn promise_input(i: &Interpreter, value: JsValue) -> JsValue {
    if matches!(value, JsValue::Object(ref object) if matches!(object.borrow().exotic, Exotic::Promise(_)))
    {
        value
    } else {
        new_promise(i, PromiseFate::Fulfilled(value))
    }
}

fn make_combinator_state(i: &mut Interpreter, count: usize, values: JsValue) -> (Obj, Obj) {
    let target = promise_object(&new_promise(i, PromiseFate::Pending)).unwrap();
    let state = Obj(Rc::new(RefCell::new(JsObject::ordinary())));
    state
        .borrow_mut()
        .set_own("target", JsValue::Object(target.clone()));
    state
        .borrow_mut()
        .set_own("remaining", JsValue::num(count as f64));
    state.borrow_mut().set_own("values", values);
    (state, target)
}

fn attach_combinator(
    i: &mut Interpreter,
    state: &Obj,
    input: JsValue,
    index: usize,
    fulfill: crate::value::NativeFn,
    reject: crate::value::NativeFn,
) -> Result<(), Throw> {
    let promise = promise_input(i, input);
    let on_fulfill = combinator_callback(i, "combinatorFulfill", fulfill, state, index);
    let on_reject = combinator_callback(i, "combinatorReject", reject, state, index);
    native_promise_then(i, promise, vec![on_fulfill, on_reject])?;
    Ok(())
}

fn native_promise_all(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let source = args
        .first()
        .ok_or_else(|| Throw::type_error("Promise.all requires an iterable"))?;
    let inputs = i.iterate(source)?;
    let values = array_values(i, vec![JsValue::Undefined; inputs.len()])?;
    let (state, target) = make_combinator_state(i, inputs.len(), values);
    if inputs.is_empty() {
        let values = state_values(&state, "values")?;
        settle_promise(&target, PromiseFate::Fulfilled(values), i)?;
    } else {
        for (index, input) in inputs.into_iter().enumerate() {
            attach_combinator(
                i,
                &state,
                input,
                index,
                native_all_fulfill,
                native_all_reject,
            )?;
        }
    }
    Ok(JsValue::Object(target))
}

fn native_promise_race(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let source = args
        .first()
        .ok_or_else(|| Throw::type_error("Promise.race requires an iterable"))?;
    let inputs = i.iterate(source)?;
    let values = array_values(i, Vec::new())?;
    let (state, target) = make_combinator_state(i, inputs.len(), values);
    for (index, input) in inputs.into_iter().enumerate() {
        attach_combinator(
            i,
            &state,
            input,
            index,
            native_race_fulfill,
            native_race_reject,
        )?;
    }
    Ok(JsValue::Object(target))
}

fn native_promise_all_settled(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let source = args
        .first()
        .ok_or_else(|| Throw::type_error("Promise.allSettled requires an iterable"))?;
    let inputs = i.iterate(source)?;
    let values = array_values(i, vec![JsValue::Undefined; inputs.len()])?;
    let (state, target) = make_combinator_state(i, inputs.len(), values);
    if inputs.is_empty() {
        let values = state_values(&state, "values")?;
        settle_promise(&target, PromiseFate::Fulfilled(values), i)?;
    } else {
        for (index, input) in inputs.into_iter().enumerate() {
            attach_combinator(
                i,
                &state,
                input,
                index,
                native_all_settled_fulfill,
                native_all_settled_reject,
            )?;
        }
    }
    Ok(JsValue::Object(target))
}

fn native_promise_any(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let source = args
        .first()
        .ok_or_else(|| Throw::type_error("Promise.any requires an iterable"))?;
    let inputs = i.iterate(source)?;
    let errors = array_values(i, vec![JsValue::Undefined; inputs.len()])?;
    let (state, target) = make_combinator_state(i, inputs.len(), errors.clone());
    state.borrow_mut().set_own("errors", errors);
    if inputs.is_empty() {
        settle_promise(
            &target,
            PromiseFate::Rejected(state_values(&state, "errors")?),
            i,
        )?;
    } else {
        for (index, input) in inputs.into_iter().enumerate() {
            attach_combinator(
                i,
                &state,
                input,
                index,
                native_any_fulfill,
                native_any_reject,
            )?;
        }
    }
    Ok(JsValue::Object(target))
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

fn map_set_raw(map: &Obj, key: MapKey, value: JsValue) {
    let mut object = map.borrow_mut();
    if let Exotic::Map(state) = &mut object.exotic {
        if let Some((_, slot)) = state.entries.iter_mut().find(|(k, _)| k.same(&key)) {
            *slot = value;
        } else {
            state.entries.push((key, value));
        }
    }
}

fn native_map(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let map = Obj(Rc::new(RefCell::new({
        let mut o = JsObject::ordinary();
        o.exotic = Exotic::Map(MapState::default());
        o.proto = Some(i.map_proto.clone());
        o
    })));
    if let Some(source) = args.first() {
        if !source.is_null() && !source.is_undefined() {
            for entry in i.iterate(source)? {
                let key = i.get_prop(&entry, "0")?;
                let value = i.get_prop(&entry, "1")?;
                map_set_raw(&map, map_key(&key), value);
            }
        }
    }
    Ok(JsValue::Object(map))
}

fn native_map_get(
    _: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let map = require_map(&this)?;
    let key = map_key(args.first().unwrap_or(&JsValue::Undefined));
    let object = map.borrow();
    if let Exotic::Map(state) = &object.exotic {
        if let Some((_, value)) = state.entries.iter().find(|(k, _)| k.same(&key)) {
            return Ok(value.clone());
        }
    }
    Ok(JsValue::Undefined)
}

fn native_map_set(
    _: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let map = require_map(&this)?;
    let key = map_key(args.first().unwrap_or(&JsValue::Undefined));
    let value = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    map_set_raw(&map, key, value);
    Ok(this)
}

fn native_map_has(
    _: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let map = require_map(&this)?;
    let key = map_key(args.first().unwrap_or(&JsValue::Undefined));
    let object = map.borrow();
    let found = matches!(&object.exotic, Exotic::Map(state) if state.entries.iter().any(|(k, _)| k.same(&key)));
    Ok(JsValue::Bool(found))
}

fn native_map_delete(
    _: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let map = require_map(&this)?;
    let key = map_key(args.first().unwrap_or(&JsValue::Undefined));
    let mut object = map.borrow_mut();
    let deleted = if let Exotic::Map(state) = &mut object.exotic {
        if let Some(pos) = state.entries.iter().position(|(k, _)| k.same(&key)) {
            state.entries.remove(pos);
            true
        } else {
            false
        }
    } else {
        false
    };
    Ok(JsValue::Bool(deleted))
}

fn native_map_clear(_: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let map = require_map(&this)?;
    if let Exotic::Map(state) = &mut map.borrow_mut().exotic {
        state.entries.clear();
    }
    Ok(JsValue::Undefined)
}

fn native_map_keys(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let map = require_map(&this)?;
    let values = if let Exotic::Map(state) = &map.borrow().exotic {
        state
            .entries
            .iter()
            .map(|(key, _)| key_value(key))
            .collect()
    } else {
        Vec::new()
    };
    array_values(i, values)
}

fn native_map_values(
    i: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let map = require_map(&this)?;
    let values = if let Exotic::Map(state) = &map.borrow().exotic {
        state
            .entries
            .iter()
            .map(|(_, value)| value.clone())
            .collect()
    } else {
        Vec::new()
    };
    array_values(i, values)
}

fn native_map_entries(
    i: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let map = require_map(&this)?;
    let entries = if let Exotic::Map(state) = &map.borrow().exotic {
        state
            .entries
            .iter()
            .map(|(key, value)| {
                array_values(i, vec![key_value(key), value.clone()]).unwrap_or(JsValue::Undefined)
            })
            .collect()
    } else {
        Vec::new()
    };
    array_values(i, entries)
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

fn native_set(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let set = Obj(Rc::new(RefCell::new({
        let mut o = JsObject::ordinary();
        o.exotic = Exotic::Set(SetState::default());
        o.proto = Some(i.set_proto.clone());
        o
    })));
    if let Some(source) = args.first() {
        if !source.is_null() && !source.is_undefined() {
            for value in i.iterate(source)? {
                let key = map_key(&value);
                if let Exotic::Set(state) = &mut set.borrow_mut().exotic {
                    if !state.entries.iter().any(|existing| existing.same(&key)) {
                        state.entries.push(key);
                    }
                }
            }
        }
    }
    Ok(JsValue::Object(set))
}

fn native_set_add(
    _: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let set = require_set(&this)?;
    let key = map_key(args.first().unwrap_or(&JsValue::Undefined));
    if let Exotic::Set(state) = &mut set.borrow_mut().exotic {
        if !state.entries.iter().any(|existing| existing.same(&key)) {
            state.entries.push(key);
        }
    }
    Ok(this)
}

fn native_set_has(
    _: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let set = require_set(&this)?;
    let key = map_key(args.first().unwrap_or(&JsValue::Undefined));
    let found = matches!(&set.borrow().exotic, Exotic::Set(state) if state.entries.iter().any(|existing| existing.same(&key)));
    Ok(JsValue::Bool(found))
}

fn native_set_delete(
    _: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let set = require_set(&this)?;
    let key = map_key(args.first().unwrap_or(&JsValue::Undefined));
    let mut object = set.borrow_mut();
    let deleted = if let Exotic::Set(state) = &mut object.exotic {
        if let Some(pos) = state
            .entries
            .iter()
            .position(|existing| existing.same(&key))
        {
            state.entries.remove(pos);
            true
        } else {
            false
        }
    } else {
        false
    };
    Ok(JsValue::Bool(deleted))
}

fn native_set_clear(_: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let set = require_set(&this)?;
    if let Exotic::Set(state) = &mut set.borrow_mut().exotic {
        state.entries.clear();
    }
    Ok(JsValue::Undefined)
}

fn native_set_values(
    i: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let set = require_set(&this)?;
    let values = if let Exotic::Set(state) = &set.borrow().exotic {
        state.entries.iter().map(key_value).collect()
    } else {
        Vec::new()
    };
    array_values(i, values)
}

fn native_set_entries(
    i: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let set = require_set(&this)?;
    let entries = if let Exotic::Set(state) = &set.borrow().exotic {
        state
            .entries
            .iter()
            .map(|key| {
                array_values(i, vec![key_value(key), key_value(key)]).unwrap_or(JsValue::Undefined)
            })
            .collect()
    } else {
        Vec::new()
    };
    array_values(i, entries)
}
fn native_object_keys(i: &mut Interpreter, _: JsValue, a: Vec<JsValue>) -> Result<JsValue, Throw> {
    let src = a.first().cloned().unwrap_or(JsValue::Undefined);
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
    if matches!(target, JsValue::Symbol(_)) { return Ok(JsValue::Undefined); }
    let JsValue::Object(object) = target else { return Err(Throw::type_error("property descriptor target is not an object")); };
    let key = i.to_property_key(args.get(1).unwrap_or(&JsValue::Undefined))?;
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
    let Some(property) = object.borrow().own_prop(&key).cloned() else { return Ok(JsValue::Undefined); };
    let mut descriptor = JsObject::ordinary();
    descriptor.proto = Some(i.object_proto.clone());
    descriptor.set_own("enumerable", JsValue::Bool(property.enumerable));
    descriptor.set_own("configurable", JsValue::Bool(property.configurable));
    if property.getter.is_some() || property.setter.is_some() {
        let accessor = |function: Option<Box<JsFunction>>| function.map(|f| {
            let mut object = JsObject::function(*f);
            object.proto = Some(i.function_proto.clone());
            let name = object.func.as_ref().map(|f| f.name.clone()).unwrap_or_default();
            let length = if name.starts_with("set ") { 1.0 } else { 0.0 };
            let _ = object.define("name", Property { value: JsValue::str(name), writable: false, enumerable: false, configurable: true, getter: None, setter: None });
            let _ = object.define("length", Property { value: JsValue::num(length), writable: false, enumerable: false, configurable: true, getter: None, setter: None });
            JsValue::object(object)
        }).unwrap_or(JsValue::Undefined);
        descriptor.set_own("get", accessor(property.getter));
        descriptor.set_own("set", accessor(property.setter));
    } else {
        descriptor.set_own("value", property.value);
        descriptor.set_own("writable", JsValue::Bool(property.writable));
    }
    Ok(JsValue::object(descriptor))
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
    let key = i.to_property_key(args.get(1).unwrap_or(&JsValue::Undefined))?;
    let value = args.get(2).cloned().unwrap_or(JsValue::Undefined);
    i.set_prop(&JsValue::Object(target), key, value)?;
    Ok(JsValue::Bool(true))
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
    reflect_target(&args)?;
    native_define_property(i, JsValue::Undefined, args)?;
    Ok(JsValue::Bool(true))
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
    if !arguments.is_object() {
        return Err(Throw::type_error("Reflect.construct argumentsList must be an object"));
    }
    let length = i.get_prop(&arguments, "length")?;
    let length = i.to_number(&length)?;
    if length > 1_048_576.0 {
        return Err(Throw::range_error("Reflect.construct argumentsList is too large"));
    }
    let count = if length.is_nan() || length <= 0.0 { 0 } else { length.floor() as usize };
    let mut values = Vec::with_capacity(count);
    for index in 0..count {
        values.push(i.get_prop(&arguments, &index.to_string())?);
    }
    i.construct_with_new_target(&target, values, new_target)
}

fn native_object_values(
    i: &mut Interpreter,
    _: JsValue,
    a: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let source = a.first().cloned().unwrap_or(JsValue::Undefined);
    let values = i
        .enumerable_keys(&source)?
        .into_iter()
        .map(|key| i.get_prop(&source, &key))
        .collect::<Result<Vec<_>, _>>()?;
    array_values(i, values)
}

fn native_object_assign(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let target = match args.first() {
        Some(JsValue::Object(_)) => args[0].clone(),
        _ => return Err(Throw::type_error("Object.assign target is not an object")),
    };
    for source in args.iter().skip(1) {
        for key in i.enumerable_property_keys(source)? {
            let value = i.get_prop(source, &key)?;
            i.set_prop(&target, &key, value)?;
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
    let base = match &this {
        JsValue::Undefined => "Undefined",
        JsValue::Null => "Null",
        JsValue::Bool(_) => "Boolean",
        JsValue::Num(_) => "Number",
        JsValue::BigInt(_) => "BigInt",
        JsValue::Str(_) => "String",
        JsValue::Symbol(_) => "Symbol",
        JsValue::Object(o) => match &o.borrow().exotic {
            Exotic::Array => "Array",
            Exotic::Map(_) => "Map",
            Exotic::Set(_) => "Set",
            Exotic::RegExp(_) => "RegExp",
            Exotic::Error => "Error",
            Exotic::Promise(_) => "Promise",
            _ => if o.borrow().func.is_some() { "Function" } else { "Object" },
        },
    };
    if matches!(this, JsValue::Undefined | JsValue::Null) {
        return Ok(JsValue::str(format!("[object {base}]")));
    }
    let tag = i.get_prop(&this, WellKnownSymbol::ToStringTag.symbol())?;
    let tag = match tag { JsValue::Str(value) => value, _ => base.into() };
    let mut units: Vec<u16> = "[object ".encode_utf16().collect();
    units.extend_from_slice(tag.units());
    units.push(b']' as u16);
    Ok(JsValue::str_units(units))
}
fn native_object_value_of(
    _: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    Ok(this)
}
fn native_array_push(
    i: &mut Interpreter,
    this: JsValue,
    a: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let mut n = i.array_length(&this);
    for v in a {
        i.set_prop(&this, &n.to_string(), v)?;
        n += 1;
    }
    Ok(JsValue::num(n as f64))
}
fn native_array_pop(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let n = i.array_length(&this);
    if n == 0 {
        return Ok(JsValue::Undefined);
    }
    let k = (n - 1).to_string();
    let v = i.get_prop(&this, &k)?;
    i.set_prop(&this, "length", JsValue::num((n - 1) as f64))?;
    if let JsValue::Object(o) = this {
        o.borrow_mut().delete_own(&k);
    }
    Ok(v)
}
fn native_array_join(
    i: &mut Interpreter,
    this: JsValue,
    a: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    if this.is_null() || this.is_undefined() {
        return Err(Throw::type_error("Array.prototype.join called on null or undefined"));
    }
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
    Ok(JsValue::num(base.powf(exponent)))
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
        let primitive = i.primitive(argument.clone())?;
        if let JsValue::BigInt(bigint) = &primitive { bigint.to_f64() }
        else { i.to_number(&primitive)? }
    } else { 0.0 };
    let value = JsValue::num(number);
    if let JsValue::Object(object) = &this {
        object.borrow_mut().set_own("[[NumberData]]", value);
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
    let value = match a.first() {
        None => JsValue::str(""),
        Some(JsValue::Symbol(symbol)) if !this.is_object() => JsValue::str_units(symbol.descriptive_units()),
        Some(value) => JsValue::str(i.to_js_string(value)?),
    };
    if let JsValue::Object(object) = &this {
        object.borrow_mut().set_own("[[StringData]]", value);
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
    let source = string_this_units(i, this)?;
    let separator = args.first().cloned().unwrap_or(JsValue::Undefined);
    let parts: Vec<crate::value::JsString> = if separator.is_undefined() {
        vec![source]
    } else {
        let separator = i.to_js_string(&separator)?;
        if separator.is_empty() {
            source.units().iter().map(|u| crate::value::JsString::from_units(vec![*u])).collect()
        } else {
            let mut result = Vec::new();
            let mut start = 0;
            while start + separator.len() <= source.len() {
                let Some(offset) = source.units()[start..].windows(separator.len())
                    .position(|part| part == separator.units()) else { break; };
                let end = start + offset;
                result.push(crate::value::JsString::from_units(source.units()[start..end].to_vec()));
                start = end + separator.len();
            }
            result.push(crate::value::JsString::from_units(source.units()[start..].to_vec()));
            result
        }
    };
    let values = parts.into_iter().map(JsValue::str).collect();
    array_values(i, values)
}

fn string_this_units(i: &mut Interpreter, this: JsValue) -> Result<crate::value::JsString, Throw> {
    if let JsValue::Object(object) = &this {
        if let Some(JsValue::Str(data)) = object.borrow().get_own("[[StringData]]") {
            return Ok(data);
        }
    }
    i.to_js_string(&this)
}

fn native_string_value_of(_: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    match this {
        JsValue::Str(_) => Ok(this),
        JsValue::Object(object) => object.borrow().get_own("[[StringData]]")
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

fn native_string_trim(
    i: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let source = string_this_units(i, this)?;
    let space = |unit: u16| char::from_u32(unit as u32).is_some_and(|ch| ch.is_whitespace());
    let start = source.units().iter().position(|unit| !space(*unit)).unwrap_or(source.len());
    let end = source.units().iter().rposition(|unit| !space(*unit)).map_or(start, |i| i + 1);
    Ok(JsValue::str_units(source.units()[start..end].to_vec()))
}

fn native_string_char_at(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let source = string_this_units(i, this)?;
    let index = string_position(i, args.first(), 0.0, source.len())?;
    Ok(source.units().get(index).map(|u| JsValue::str_units(vec![*u]))
        .unwrap_or_else(|| JsValue::str("")))
}

fn native_string_char_code_at(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let source = string_this_units(i, this)?;
    let index = string_position(i, args.first(), 0.0, source.len())?;
    Ok(JsValue::num(source.units().get(index).map_or(f64::NAN, |u| *u as f64)))
}

fn native_string_code_point_at(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let source = string_this_units(i, this)?;
    let index = string_position(i, args.first(), 0.0, source.len())?;
    let Some(&first) = source.units().get(index) else { return Ok(JsValue::Undefined); };
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
    let start = string_position(i, args.first(), 0.0, source.len())?;
    let end = string_position(i, args.get(1), source.len() as f64, source.len())?;
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
    let left = string_this_units(i, this)?;
    let right = i.to_js_string(args.first().unwrap_or(&JsValue::Undefined))?;
    Ok(JsValue::num(match left.units().cmp(right.units()) { std::cmp::Ordering::Less => -1.0, std::cmp::Ordering::Equal => 0.0, std::cmp::Ordering::Greater => 1.0 }))
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
    let needle = i.to_js_string(args.first().unwrap_or(&JsValue::Undefined))?;
    Ok(JsValue::Bool(needle.is_empty() || source.units().windows(needle.len()).any(|part| part == needle.units())))
}

fn native_string_replace(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let source = string_this_units(i, this)?;
    if args.first().is_some_and(|v| regexp_data(v).is_some()) {
        return regexp_replace(i, &source, &args);
    }
    let search = i.to_js_string(args.first().unwrap_or(&JsValue::Undefined))?;
    let replacement = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    let position = if search.is_empty() { Some(0) } else {
        source.units().windows(search.len()).position(|part| part == search.units())
    };
    if let Some(position) = position {
        let replacement = if callable_function(&replacement).is_some() {
            let result = i.call_value(
                &replacement,
                JsValue::Undefined,
                vec![JsValue::str(search.clone())],
            )?;
            i.to_js_string(&result)?
        } else {
            i.to_js_string(&replacement)?
        };
        let mut output = Vec::with_capacity(source.len() + replacement.len());
        output.extend_from_slice(&source.units()[..position]);
        output.extend_from_slice(replacement.units());
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
    let search = i.to_js_string(args.first().unwrap_or(&JsValue::Undefined))?;
    Ok(JsValue::Bool(source.units().starts_with(search.units())))
}

fn native_string_ends_with(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let source = string_this_units(i, this)?;
    let search = i.to_js_string(args.first().unwrap_or(&JsValue::Undefined))?;
    Ok(JsValue::Bool(source.units().ends_with(search.units())))
}

fn array_callback(
    i: &mut Interpreter,
    this: JsValue,
    args: &[JsValue],
    mut each: impl FnMut(&mut Interpreter, usize, JsValue, JsValue) -> Result<(), Throw>,
) -> Result<JsValue, Throw> {
    let callback = args
        .first()
        .ok_or_else(|| Throw::type_error("array callback is not a function"))?;
    if callable_function(callback).is_none() {
        return Err(Throw::type_error("array callback is not a function"));
    }
    let legacy = match &this {
        JsValue::Object(object) => i.legacy_for(object),
        _ => None,
    };
    let length = match &legacy {
        Some((handle, hooks)) => (hooks.supported_index_count)(*handle)? as usize,
        None => i.array_length(&this),
    };
    for index in 0..length {
        if let JsValue::Object(object) = &this {
            if !i.legacy_has_property(object, index.to_string())? { continue; }
        }
        let value = i.get_prop(&this, &index.to_string())?;
        let result = i.call_value(
            callback,
            args.get(1).cloned().unwrap_or(JsValue::Undefined),
            vec![value.clone(), JsValue::num(index as f64), this.clone()],
        )?;
        each(i, index, value, result)?;
    }
    Ok(JsValue::Undefined)
}

fn native_array_map(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let out = JsValue::object({
        let mut object = JsObject::array();
        object.proto = Some(i.array_proto.clone());
        object
    });
    array_callback(i, this, &args, |i, index, _, result| {
        i.set_prop(&out, &index.to_string(), result)
    })?;
    Ok(out)
}

fn native_array_filter(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let out = JsValue::object({
        let mut object = JsObject::array();
        object.proto = Some(i.array_proto.clone());
        object
    });
    let mut next = 0usize;
    array_callback(i, this, &args, |i, _, value, selected| {
        if selected.to_boolean() {
            i.set_prop(&out, &next.to_string(), value)?;
            next += 1;
        }
        Ok(())
    })?;
    Ok(out)
}

fn native_array_for_each(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    array_callback(i, this, &args, |_, _, _, _| Ok(()))
}

fn native_array_reduce(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let callback = args
        .first()
        .ok_or_else(|| Throw::type_error("array reducer is not a function"))?;
    if callable_function(callback).is_none() {
        return Err(Throw::type_error("array reducer is not a function"));
    }
    let length = i.array_length(&this);
    let mut index = 0usize;
    let mut accumulator = if args.len() > 1 {
        args[1].clone()
    } else if length > 0 {
        let value = i.get_prop(&this, "0")?;
        index = 1;
        value
    } else {
        return Err(Throw::type_error(
            "reduce of empty array with no initial value",
        ));
    };
    while index < length {
        let value = i.get_prop(&this, &index.to_string())?;
        accumulator = i.call_value(
            callback,
            JsValue::Undefined,
            vec![accumulator, value, JsValue::num(index as f64), this.clone()],
        )?;
        index += 1;
    }
    Ok(accumulator)
}

fn native_array_slice(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let length = i.array_length(&this) as i64;
    let start = i
        .to_number(args.first().unwrap_or(&JsValue::num(0.0)))?
        .trunc() as i64;
    let end = i
        .to_number(args.get(1).unwrap_or(&JsValue::num(length as f64)))?
        .trunc() as i64;
    let start = if start < 0 {
        (length + start).max(0)
    } else {
        start.min(length)
    } as usize;
    let end = if end < 0 {
        (length + end).max(0)
    } else {
        end.min(length)
    } as usize;
    let mut values = Vec::new();
    for index in start..end.max(start) {
        values.push(i.get_prop(&this, &index.to_string())?);
    }
    array_values(i, values)
}

fn native_array_concat(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let mut values = i.iterate(&this)?;
    for argument in args {
        if matches!(argument, JsValue::Object(ref object) if matches!(object.borrow().exotic, Exotic::Array))
            || matches!(argument, JsValue::Str(_))
        {
            values.extend(i.iterate(&argument)?);
        } else {
            values.push(argument);
        }
    }
    array_values(i, values)
}

fn native_array_includes(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let search = args.first().cloned().unwrap_or(JsValue::Undefined);
    Ok(JsValue::Bool(
        i.iterate(&this)?.into_iter().any(|value| value == search),
    ))
}

fn native_array_index_of(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let search = args.first().cloned().unwrap_or(JsValue::Undefined);
    for (index, value) in i.iterate(&this)?.into_iter().enumerate() {
        if value == search {
            return Ok(JsValue::num(index as f64));
        }
    }
    Ok(JsValue::num(-1.0))
}
fn native_boolean(_: &mut Interpreter, this: JsValue, a: Vec<JsValue>) -> Result<JsValue, Throw> {
    let value = JsValue::Bool(a.first().map(|x| x.to_boolean()).unwrap_or(false));
    if let JsValue::Object(object) = &this {
        object.borrow_mut().set_own("[[BooleanData]]", value);
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
fn native_function_constructor(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let mut parts = args
        .iter()
        .map(|a| i.to_string(a))
        .collect::<Result<Vec<_>, _>>()?;
    let body = parts.pop().unwrap_or_default();
    let source = format!("function anonymous({}) {{\n{}\n}}", parts.join(","), body);
    let script = crate::parse_script(&source).map_err(|e| Throw::syntax_error(e.to_string()))?;
    if script.body.len() != 1 {
        return Err(Throw::syntax_error("invalid Function constructor source"));
    }
    if let ast::StmtKind::Function(function) = &script.body[0].kind {
        Ok(i.make_function(function, &i.global_env))
    } else {
        Err(Throw::syntax_error("invalid Function constructor source"))
    }
}
fn native_has_own_property(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    native_object_has_own(
        i,
        JsValue::Undefined,
        vec![this, args.first().cloned().unwrap_or(JsValue::Undefined)],
    )
}

fn native_property_is_enumerable(
    i: &mut Interpreter, this: JsValue, args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let JsValue::Object(object) = this else { return Err(Throw::type_error("propertyIsEnumerable receiver is not an object")); };
    let key = i.to_property_key(args.first().unwrap_or(&JsValue::Undefined))?;
    if let Some(property) = binary_index_descriptor(&object, &key) { return Ok(JsValue::Bool(property.is_some())); }
    let property = match key.as_string() {
        Some(name) => i.legacy_own_property(&object, name, false)?,
        None => object.borrow().own_prop(&key).cloned(),
    };
    Ok(JsValue::Bool(property.is_some_and(|property| property.enumerable)))
}

fn native_array_some(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    if this.is_null() || this.is_undefined() {
        return Err(Throw::type_error("Array.some receiver is nullish"));
    }
    let length = i.get_prop(&this, "length")?;
    let length = i.to_number(&length)?;
    let length = if length.is_nan() || length <= 0.0 {
        0.0
    } else {
        length.floor()
    };
    if length > 1_048_576.0 {
        return Err(Throw::range_error("Array.some length limit"));
    }
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    if callable_function(&callback).is_none() {
        return Err(Throw::type_error("Array.some callback is not callable"));
    }
    let receiver = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    for index in 0..length as usize {
        let key = index.to_string();
        let present = match &this {
            JsValue::Object(o) => i.legacy_has_property(o, &key)?,
            JsValue::Str(s) => index < s.len(),
            _ => false,
        };
        if present {
            let value = i.get_prop(&this, &key)?;
            if i.call_value(
                &callback,
                receiver.clone(),
                vec![value, JsValue::num(index as f64), this.clone()],
            )?
            .to_boolean()
            {
                return Ok(JsValue::Bool(true));
            }
        }
    }
    Ok(JsValue::Bool(false))
}
