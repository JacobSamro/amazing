// Private per-realm AbstractModuleSource intrinsic and host compiled-source factory.
// The native brand is a real parsed module record, never an authored property.

impl Interpreter {
    fn install_abstract_module_source(&mut self) {
        let mut prototype = JsObject::ordinary();
        prototype.proto = Some(self.object_proto.clone());
        let prototype = Obj(Rc::new(RefCell::new(prototype)));
        let constructor = Self::native_constructor_arity("AbstractModuleSource", native_abstract_module_source, &self.function_proto, 0);
        let JsValue::Object(ctor) = &constructor else { unreachable!() };
        ctor.borrow_mut().delete_own("prototype");
        ctor.borrow_mut().define("prototype", symbol_data(JsValue::Object(prototype.clone()), false, false)).unwrap();
        prototype.borrow_mut().define("constructor", symbol_data(constructor.clone(), true, true)).unwrap();
        let getter = Self::native_arity("get [Symbol.toStringTag]", native_module_source_tag, &self.function_proto, 0);
        prototype.borrow_mut().define(WellKnownSymbol::ToStringTag.symbol(), Property::accessor(Some(getter), None)).unwrap();
        self.intrinsic_constructors.insert("AbstractModuleSource".into(), (ctor.clone(), prototype.clone()));
        // A host-defined source subclass has this prototype's prototype equal
        // to %AbstractModuleSource.prototype%. Its factory is native Rust only.
        let mut compiled = JsObject::ordinary();
        compiled.proto = Some(prototype);
        let compiled = Obj(Rc::new(RefCell::new(compiled)));
        self.intrinsic_constructors.insert("CompiledModuleSource".into(), (ctor.clone(), compiled));
    }

    /// Actual current realm intrinsic, for hosts implementing the Test262 contract.
    /// It is deliberately absent from the realm's global object.
    pub fn binding_abstract_module_source_constructor(&self) -> JsValue {
        JsValue::Object(self.intrinsic_constructors["AbstractModuleSource"].0.clone())
    }

    /// Allocate the host-defined source object in the actual active module realm.
    /// The linker retains one object for each canonical record in that realm.
    pub fn binding_create_compiled_module_source(&self, record: Rc<crate::module::CompiledModuleSource>) -> JsValue {
        let mut object = JsObject::ordinary();
        object.proto = Some(self.intrinsic_constructors["CompiledModuleSource"].1.clone());
        object.realm = Some(self.binding_current_realm().realm);
        object.exotic = Exotic::ModuleSource(record);
        JsValue::object(object)
    }
}
fn native_abstract_module_source(_: &mut Interpreter, _: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    Err(Throw::type_error("AbstractModuleSource cannot be called or constructed"))
}
fn native_module_source_tag(_: &mut Interpreter, receiver: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let JsValue::Object(object) = receiver else { return Ok(JsValue::Undefined); };
    let object = object.borrow();
    match &object.exotic {
        Exotic::ModuleSource(record) => Ok(JsValue::str(record.kind())),
        _ => Ok(JsValue::Undefined),
    }
}
