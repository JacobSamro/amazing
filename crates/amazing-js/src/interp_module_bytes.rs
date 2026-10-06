// Byte-module allocation uses real saved realm intrinsics, never mutable globals.
impl Interpreter {
    pub(crate) fn binding_create_module_bytes(&mut self, bytes: Vec<u8>) -> Result<JsValue, Throw> {
        let buffer_proto = self.intrinsic_constructors.get("ArrayBuffer")
            .map(|(_, prototype)| prototype.clone())
            .ok_or_else(|| Throw::type_error("missing intrinsic ArrayBuffer prototype"))?;
        let view_proto = self.intrinsic_constructors.get("Uint8Array")
            .map(|(_, prototype)| prototype.clone())
            .ok_or_else(|| Throw::type_error("missing intrinsic Uint8Array prototype"))?;
        let length = bytes.len();
        let mut buffer = JsObject::ordinary();
        buffer.proto = Some(buffer_proto);
        let buffer = Obj(Rc::new(RefCell::new(buffer)));
        binary_register(&buffer, BinaryState::Buffer(Rc::new(RefCell::new(BinaryBuffer {
            bytes, max: length, resizable: false, shared: false, detached: false, immutable: true,
        }))));
        let mut view = JsObject::ordinary();
        view.proto = Some(view_proto);
        let view = Obj(Rc::new(RefCell::new(view)));
        self.binary_view_on(view, buffer, 0, Some(length), Some(BinaryKind::Uint8))
    }
}
