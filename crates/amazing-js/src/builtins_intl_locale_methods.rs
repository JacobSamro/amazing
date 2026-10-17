// Locale-sensitive methods on the existing language built-ins.

impl Interpreter {
    fn install_intl_locale_methods(&mut self) {
        intl_method(self, &self.array_proto, "toLocaleString", 0, intl_array_to_locale_string);
        intl_method(self, &self.date_proto, "toLocaleString", 0, intl_date_to_locale_string);
        intl_method(self, &self.date_proto, "toLocaleDateString", 0, intl_date_to_locale_string);
        intl_method(self, &self.date_proto, "toLocaleTimeString", 0, intl_date_to_locale_string);
        if let Some(number) = self.global_env.resolve("Number") {
            if let Ok(JsValue::Object(proto)) = self.get_prop(&number, "prototype") {
                intl_method(self, &proto, "toLocaleString", 0, intl_number_to_locale_string);
            }
        }
    }
}

fn intl_construct_service(i: &mut Interpreter, name: &str, locales: JsValue, options: JsValue) -> Result<JsValue, Throw> {
    // These language-builtin bridges (String.prototype.localeCompare,
    // Number/BigInt.prototype.toLocaleString, Date.prototype.toLocaleString) use
    // the %Collator%/%NumberFormat%/%DateTimeFormat% intrinsics directly per
    // ECMA-402, not whatever value the mutable `Intl.<name>` property currently
    // holds -- testIntl.js's taintDataProperty helper replaces that property
    // with a throwing/undefined accessor and expects these methods to be
    // unaffected (taint-Intl-Collator.js, taint-Intl-NumberFormat.js,
    // taint-Intl-DateTimeFormat.js).
    let constructor = i.intrinsic_constructors.get(name)
        .map(|(constructor, _)| JsValue::Object(constructor.clone()))
        .ok_or_else(|| Throw::type_error("Intl service unavailable"))?;
    i.construct(&constructor, vec![locales, options])
}

fn intl_string_locale_compare(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let left = string_this_units(i, this)?.to_utf8_lossy();
    let right = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    let locales = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    let options = args.get(2).cloned().unwrap_or(JsValue::Undefined);
    let collator = intl_construct_service(i, "Collator", locales, options)?;
    let compare = i.get_prop(&collator, "compare")?;
    i.call_value(&compare, JsValue::Undefined, vec![JsValue::str(left), JsValue::str(right)])
}

fn intl_number_to_locale_string(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let number = match this {
        JsValue::Num(number) => number,
        JsValue::Object(object) => match object.borrow().boxed_primitive.clone() {
            Some(JsValue::Num(number)) => number,
            _ => return Err(Throw::type_error("incompatible Number receiver")),
        },
        _ => return Err(Throw::type_error("incompatible Number receiver")),
    };
    let locales = args.first().cloned().unwrap_or(JsValue::Undefined);
    let options = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    let formatter = intl_construct_service(i, "NumberFormat", locales, options)?;
    let format = i.get_prop(&formatter, "format")?;
    i.call_value(&format, JsValue::Undefined, vec![JsValue::num(number)])
}

fn intl_date_to_locale_string(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let JsValue::Object(object) = &this else { return Err(Throw::type_error("incompatible Date receiver")); };
    let Some(JsValue::Num(timestamp)) = object.borrow().get_own("[[DateValue]]") else {
        return Err(Throw::type_error("incompatible Date receiver"));
    };
    if !timestamp.is_finite() { return Ok(JsValue::str("Invalid Date")); }
    let name = binary_current_name(i);
    let locales = args.first().cloned().unwrap_or(JsValue::Undefined);
    let options = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    // ToDateTimeOptions(options, required, defaults): the options object is read for the
    // required fields; when none is present the numeric defaults are layered on a child object.
    let (required, defaults) = match name.as_str() {
        "toLocaleDateString" => ("date", "date"),
        "toLocaleTimeString" => ("time", "time"),
        _ => ("any", "all"),
    };
    let options = if options.is_undefined() {
        // ECMA-402 builds default options with a null prototype, so inherited
        // Object.prototype properties must not leak into the formatter.
        JsValue::object(JsObject::ordinary())
    } else if options.is_null() {
        return Err(Throw::type_error("options must not be null"));
    } else { array_receiver(i, options)? };
    let mut need_defaults = true;
    let date_fields: &[&str] = &["weekday", "year", "month", "day"];
    let time_fields: &[&str] = &["dayPeriod", "hour", "minute", "second", "fractionalSecondDigits"];
    for (group, fields) in [("date", date_fields), ("time", time_fields)] {
        if required != "any" && required != group { continue; }
        for field in fields {
            if !i.get_prop(&options, *field)?.is_undefined() { need_defaults = false; }
        }
    }
    let date_style = i.get_prop(&options, "dateStyle")?;
    let time_style = i.get_prop(&options, "timeStyle")?;
    if !date_style.is_undefined() || !time_style.is_undefined() {
        need_defaults = false;
        if required == "date" && !time_style.is_undefined() {
            return Err(Throw::type_error("timeStyle is not allowed in toLocaleDateString"));
        }
        if required == "time" && !date_style.is_undefined() {
            return Err(Throw::type_error("dateStyle is not allowed in toLocaleTimeString"));
        }
    }
    let options = if need_defaults {
        let JsValue::Object(parent) = &options else { unreachable!() };
        let mut child = JsObject::ordinary();
        child.proto = Some(parent.clone());
        let child = JsValue::object(child);
        if matches!(defaults, "date" | "all") {
            for field in ["year", "month", "day"] { i.set_prop(&child, field, JsValue::str("numeric"))?; }
        }
        if matches!(defaults, "time" | "all") {
            for field in ["hour", "minute", "second"] { i.set_prop(&child, field, JsValue::str("numeric"))?; }
        }
        child
    } else { options };
    let formatter = intl_construct_service(i, "DateTimeFormat", locales, options)?;
    let format = i.get_prop(&formatter, "format")?;
    i.call_value(&format, JsValue::Undefined, vec![JsValue::num(timestamp)])
}

fn intl_array_to_locale_string(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let JsValue::Object(_) = &this else { return Err(Throw::type_error("incompatible Array receiver")); };
    let length = i.get_prop(&this, "length")?;
    let length = i.to_number(&length)?.max(0.0).min(1_000_000.0) as usize;
    let mut strings = Vec::with_capacity(length);
    for index in 0..length {
        let value = i.get_prop(&this, index.to_string())?;
        if matches!(value, JsValue::Null | JsValue::Undefined) {
            strings.push(String::new());
            continue;
        }
        let method = i.get_prop(&value, "toLocaleString")?;
        if callable_function(&method).is_none() {
            return Err(Throw::type_error("element toLocaleString is not callable"));
        }
        // ECMA-402 Array.prototype.toLocaleString passes exactly (locales, options).
        let forwarded = vec![args.first().cloned().unwrap_or(JsValue::Undefined), args.get(1).cloned().unwrap_or(JsValue::Undefined)];
        let text = i.call_value(&method, value, forwarded)?;
        strings.push(i.to_string(&text)?);
    }
    Ok(JsValue::str(strings.join(",")))
}

#[cfg(test)]
mod intl_locale_method_tests {
    use super::*;

    #[test]
    fn language_builtins_use_intl_formatters() {
        let mut i = Interpreter::new();
        let value = i.run_script("[12345.5.toLocaleString('de'), new Date(0).toLocaleDateString('en'), 'ä'.localeCompare('a','de',{sensitivity:'base'})].join('|')").unwrap();
        assert_eq!(i.to_string(&value).unwrap(), "12.345,5|1/1/1970|0");
    }
}
