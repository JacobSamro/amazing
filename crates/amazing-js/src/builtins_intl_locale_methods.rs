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
    let mode = match name.as_str() {
        "toLocaleDateString" => ("date", "date"),
        "toLocaleTimeString" => ("time", "time"),
        _ => ("any", "all"),
    };
    INTL_DATE_MODE.with(|slot| slot.set(Some(mode)));
    let formatter = intl_construct_service(i, "DateTimeFormat", locales, options);
    INTL_DATE_MODE.with(|slot| slot.set(None));
    let formatter = formatter?;
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
