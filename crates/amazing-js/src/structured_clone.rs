// HTML StructuredSerialize / StructuredDeserialize. The wire graph owns every
// byte and uses node indices for cycles and repeated object identity.

#[derive(Clone, Debug)]
pub enum SerializedAtom {
    Undefined,
    Null,
    Boolean(bool),
    Number(f64),
    BigInt { negative: bool, limbs: Vec<u32> },
    String(Vec<u16>),
    Reference(usize),
}

#[derive(Clone, Debug)]
pub enum SerializedBoxKind {
    Boolean,
    Number,
    BigInt,
    String,
}

#[derive(Clone, Debug)]
pub struct SerializedHostValue {
    pub type_name: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
pub enum SerializedNode {
    Object {
        properties: Vec<(Vec<u16>, SerializedAtom)>,
    },
    Array {
        length: usize,
        properties: Vec<(Vec<u16>, SerializedAtom)>,
    },
    Boxed {
        kind: SerializedBoxKind,
        value: SerializedAtom,
    },
    Date(f64),
    RegExp {
        pattern: Vec<u16>,
        flags: String,
    },
    ArrayBuffer {
        bytes: Vec<u8>,
        max_byte_length: usize,
        resizable: bool,
    },
    ArrayBufferView {
        buffer: usize,
        byte_offset: usize,
        byte_length: Option<usize>,
        kind: Option<String>,
    },
    Map(Vec<(SerializedAtom, SerializedAtom)>),
    Set(Vec<SerializedAtom>),
    Error {
        name: String,
        message: Option<Vec<u16>>,
        cause: Option<SerializedAtom>,
    },
    Host(SerializedHostValue),
}

#[derive(Clone, Debug)]
pub struct SerializedValue {
    pub root: SerializedAtom,
    pub nodes: Vec<SerializedNode>,
}

/// An embedding can add [Serializable] platform objects without introducing
/// process-local handles into the serialized graph.
pub trait StructuredCloneHost {
    fn serialize_host(&mut self, _value: &JsValue) -> Result<Option<SerializedHostValue>, Throw> {
        Ok(None)
    }
    fn deserialize_host(
        &mut self,
        _value: &SerializedHostValue,
        _target: &mut Interpreter,
    ) -> Result<JsValue, Throw> {
        Err(Throw::error("DataCloneError", "unsupported host object"))
    }
}

struct NoStructuredCloneHost;
impl StructuredCloneHost for NoStructuredCloneHost {}

struct StructuredSerializer<'a> {
    interpreter: &'a mut Interpreter,
    host: &'a mut dyn StructuredCloneHost,
    nodes: Vec<SerializedNode>,
    memory: HashMap<usize, usize>,
    for_storage: bool,
}

impl StructuredSerializer<'_> {
    fn atom(&mut self, value: &JsValue) -> Result<SerializedAtom, Throw> {
        match value {
            JsValue::Undefined => Ok(SerializedAtom::Undefined),
            JsValue::Null => Ok(SerializedAtom::Null),
            JsValue::Bool(value) => Ok(SerializedAtom::Boolean(*value)),
            JsValue::Num(value) => Ok(SerializedAtom::Number(*value)),
            JsValue::BigInt(value) => Ok(SerializedAtom::BigInt {
                negative: value.negative,
                limbs: value.limbs.clone(),
            }),
            JsValue::Str(value) => Ok(SerializedAtom::String(value.units().to_vec())),
            JsValue::Symbol(_) => Err(self
                .interpreter
                .structured_clone_error("Symbol cannot be cloned")),
            JsValue::Object(object) => {
                let identity = Rc::as_ptr(&object.0) as usize;
                if let Some(index) = self.memory.get(&identity) {
                    return Ok(SerializedAtom::Reference(*index));
                }
                let index = self.nodes.len();
                self.memory.insert(identity, index);
                self.nodes.push(SerializedNode::Object {
                    properties: Vec::new(),
                });
                let node = self.object(value, object)?;
                self.nodes[index] = node;
                Ok(SerializedAtom::Reference(index))
            }
        }
    }

    fn properties(
        &mut self,
        value: &JsValue,
        object: &Obj,
    ) -> Result<Vec<(Vec<u16>, SerializedAtom)>, Throw> {
        let keys = object.borrow().own_keys_enumerable();
        let mut properties = Vec::with_capacity(keys.len());
        for key in keys {
            if object.borrow().own_prop(&key).is_none() {
                continue;
            }
            let child = self.interpreter.get_prop(value, key.clone())?;
            properties.push((key.units().to_vec(), self.atom(&child)?));
        }
        Ok(properties)
    }

    fn object(&mut self, value: &JsValue, object: &Obj) -> Result<SerializedNode, Throw> {
        if let Some(state) = binary_state(object) {
            return match state {
                BinaryState::Buffer(buffer) => {
                    let buffer = buffer.borrow();
                    if buffer.detached || buffer.shared || (self.for_storage && buffer.shared) {
                        return Err(self
                            .interpreter
                            .structured_clone_error("ArrayBuffer cannot be cloned"));
                    }
                    Ok(SerializedNode::ArrayBuffer {
                        bytes: buffer.bytes.clone(),
                        max_byte_length: buffer.max,
                        resizable: buffer.resizable,
                    })
                }
                BinaryState::View(view) => {
                    if binary_view_out_of_bounds(&view) {
                        return Err(self
                            .interpreter
                            .structured_clone_error("out-of-bounds ArrayBuffer view"));
                    }
                    let SerializedAtom::Reference(buffer) =
                        self.atom(&JsValue::Object(view.buffer_object.clone()))?
                    else {
                        unreachable!()
                    };
                    Ok(SerializedNode::ArrayBufferView {
                        buffer,
                        byte_offset: view.offset,
                        byte_length: view.length,
                        kind: view.kind.map(|kind| kind.name().to_string()),
                    })
                }
            };
        }

        let (callable, host_object, exotic, boxed, date) = {
            let object = object.borrow();
            let boxed = [
                ("[[BooleanData]]", SerializedBoxKind::Boolean),
                ("[[NumberData]]", SerializedBoxKind::Number),
                ("[[BigIntData]]", SerializedBoxKind::BigInt),
                ("[[StringData]]", SerializedBoxKind::String),
            ]
            .into_iter()
            .find_map(|(key, kind)| object.get_own(key).map(|value| (kind, value)));
            (
                object.func.is_some(),
                object.host_identity.is_some(),
                object.exotic.clone(),
                boxed,
                object.get_own("[[DateValue]]"),
            )
        };
        if host_object {
            return self
                .host
                .serialize_host(value)?
                .map(SerializedNode::Host)
                .ok_or_else(|| {
                    self.interpreter
                        .structured_clone_error("host object is not serializable")
                });
        }
        if callable {
            return Err(self
                .interpreter
                .structured_clone_error("Function cannot be cloned"));
        }
        if let Some((kind, primitive)) = boxed {
            return Ok(SerializedNode::Boxed {
                kind,
                value: self.atom(&primitive)?,
            });
        }
        if let Some(JsValue::Num(date)) = date {
            return Ok(SerializedNode::Date(date));
        }
        match exotic {
            Exotic::Ordinary => Ok(SerializedNode::Object {
                properties: self.properties(value, object)?,
            }),
            Exotic::Array => Ok(SerializedNode::Array {
                length: self.interpreter.array_length(value),
                properties: self.properties(value, object)?,
            }),
            Exotic::RegExp(regex) => Ok(SerializedNode::RegExp {
                pattern: regex.pattern_utf16().to_vec(),
                flags: regex.flags().as_string(),
            }),
            Exotic::Map(state) => {
                let mut entries = Vec::with_capacity(state.entries.len());
                for (key, value) in &state.entries {
                    entries.push((self.atom(&key_value(key))?, self.atom(value)?));
                }
                Ok(SerializedNode::Map(entries))
            }
            Exotic::Set(state) => {
                let mut entries = Vec::with_capacity(state.entries.len());
                for key in &state.entries {
                    entries.push(self.atom(&key_value(key))?);
                }
                Ok(SerializedNode::Set(entries))
            }
            Exotic::Error => {
                let name = self.interpreter.get_prop(value, "name")?;
                let name = match name {
                    JsValue::Str(name)
                        if matches!(
                            name.to_utf8_lossy().as_str(),
                            "Error"
                                | "EvalError"
                                | "RangeError"
                                | "ReferenceError"
                                | "SyntaxError"
                                | "TypeError"
                                | "URIError"
                        ) =>
                    {
                        name.to_utf8_lossy()
                    }
                    _ => "Error".to_string(),
                };
                let (message, cause) = {
                    let object = object.borrow();
                    (object.get_own("message"), object.get_own("cause"))
                };
                let message = message
                    .map(|value| self.interpreter.to_js_string(&value))
                    .transpose()?
                    .map(|value| value.units().to_vec());
                let cause = cause.map(|value| self.atom(&value)).transpose()?;
                Ok(SerializedNode::Error {
                    name,
                    message,
                    cause,
                })
            }
            _ => Err(self
                .interpreter
                .structured_clone_error("object is not serializable")),
        }
    }
}

fn structured_atom_value(
    atom: &SerializedAtom,
    objects: &[Option<JsValue>],
) -> Result<JsValue, Throw> {
    Ok(match atom {
        SerializedAtom::Undefined => JsValue::Undefined,
        SerializedAtom::Null => JsValue::Null,
        SerializedAtom::Boolean(value) => JsValue::Bool(*value),
        SerializedAtom::Number(value) => JsValue::Num(*value),
        SerializedAtom::BigInt { negative, limbs } => {
            JsValue::BigInt(Rc::new(BinaryBigInt::new(*negative, limbs.clone())))
        }
        SerializedAtom::String(value) => JsValue::str_units(value.clone()),
        SerializedAtom::Reference(index) => objects
            .get(*index)
            .and_then(Clone::clone)
            .ok_or_else(|| Throw::error("DataCloneError", "invalid serialized reference"))?,
    })
}

impl Interpreter {
    fn structured_clone_error(&mut self, message: &str) -> Throw {
        if let Some(constructor) = self.global_env.resolve("DOMException") {
            if let Ok(value) = self.construct(
                &constructor,
                vec![JsValue::str(message), JsValue::str("DataCloneError")],
            ) {
                return Throw(value);
            }
        }
        Throw::error("DataCloneError", message)
    }

    fn install_structured_clone(&mut self) {
        // The structured-clone Error family includes EvalError. The shared
        // Error installer does not yet install that constructor.
        let eval_error = Self::native("EvalError", native_error, &self.function_proto);
        if let JsValue::Object(function) = &eval_error {
            if let Some(JsValue::Object(prototype)) = function.borrow().get_own("prototype") {
                prototype.borrow_mut().proto = self
                    .global_env
                    .resolve("Error")
                    .and_then(|error| {
                        if let JsValue::Object(object) = error {
                            object.borrow().get_own("prototype")
                        } else {
                            None
                        }
                    })
                    .and_then(|value| {
                        if let JsValue::Object(object) = value {
                            Some(object)
                        } else {
                            None
                        }
                    });
                prototype
                    .borrow_mut()
                    .set_own("name", JsValue::str("EvalError"));
                prototype.borrow_mut().set_own("message", JsValue::str(""));
                prototype
                    .borrow_mut()
                    .set_own("constructor", eval_error.clone());
            }
        }
        self.define_global("EvalError", eval_error);
        let function = Self::native(
            "structuredClone",
            native_structured_clone,
            &self.function_proto,
        );
        if let JsValue::Object(object) = &function {
            object.borrow_mut().constructible = false;
            object.borrow_mut().delete_own("prototype");
            object
                .borrow_mut()
                .define("length", symbol_data(JsValue::num(1.0), false, true))
                .unwrap();
        }
        self.define_global("structuredClone", function.clone());
        self.global_obj
            .borrow_mut()
            .define("structuredClone", symbol_data(function, true, true))
            .unwrap();
    }

    pub fn structured_serialize(&mut self, value: &JsValue) -> Result<SerializedValue, Throw> {
        self.structured_serialize_with_transfer(value, &[])
    }

    pub fn structured_serialize_for_storage(
        &mut self,
        value: &JsValue,
    ) -> Result<SerializedValue, Throw> {
        let mut host = NoStructuredCloneHost;
        self.structured_serialize_with_host(value, &[], true, &mut host)
    }

    pub fn structured_serialize_with_transfer(
        &mut self,
        value: &JsValue,
        transfer: &[JsValue],
    ) -> Result<SerializedValue, Throw> {
        let mut host = NoStructuredCloneHost;
        self.structured_serialize_with_host(value, transfer, false, &mut host)
    }

    pub fn structured_serialize_with_host(
        &mut self,
        value: &JsValue,
        transfer: &[JsValue],
        for_storage: bool,
        host: &mut dyn StructuredCloneHost,
    ) -> Result<SerializedValue, Throw> {
        if for_storage && !transfer.is_empty() {
            return Err(self.structured_clone_error("storage serialization cannot transfer"));
        }
        let mut seen = std::collections::HashSet::new();
        let mut buffers = Vec::new();
        for entry in transfer {
            let JsValue::Object(object) = entry else {
                return Err(self.structured_clone_error("transfer entry is not an object"));
            };
            if !seen.insert(Rc::as_ptr(&object.0) as usize) {
                return Err(self.structured_clone_error("duplicate transfer entry"));
            }
            let Some(BinaryState::Buffer(buffer)) = binary_state(object) else {
                return Err(self.structured_clone_error("object is not transferable"));
            };
            if {
                let buffer = buffer.borrow();
                buffer.detached || buffer.shared || buffer.immutable
            } {
                return Err(self.structured_clone_error("ArrayBuffer cannot be transferred"));
            }
            buffers.push((Rc::as_ptr(&object.0) as usize, buffer));
        }
        let mut serializer = StructuredSerializer {
            interpreter: self,
            host,
            nodes: Vec::new(),
            memory: HashMap::new(),
            for_storage,
        };
        let root = serializer.atom(value)?;
        if buffers
            .iter()
            .any(|(identity, _)| !serializer.memory.contains_key(identity))
        {
            return Err(serializer
                .interpreter
                .structured_clone_error("unreachable transfer entry"));
        }
        let result = SerializedValue {
            root,
            nodes: serializer.nodes,
        };
        for (_, buffer) in buffers {
            let mut buffer = buffer.borrow_mut();
            buffer.bytes.clear();
            buffer.detached = true;
        }
        Ok(result)
    }

    pub fn structured_deserialize(
        &mut self,
        serialized: &SerializedValue,
    ) -> Result<JsValue, Throw> {
        let mut host = NoStructuredCloneHost;
        self.structured_deserialize_with_host(serialized, &mut host)
    }

    pub fn structured_deserialize_with_host(
        &mut self,
        serialized: &SerializedValue,
        host: &mut dyn StructuredCloneHost,
    ) -> Result<JsValue, Throw> {
        let mut objects = vec![None; serialized.nodes.len()];
        // Allocate before filling graph edges. ArrayBuffer views are allocated
        // in a second pass, after every backing buffer has an identity.
        for (index, node) in serialized.nodes.iter().enumerate() {
            let value = match node {
                SerializedNode::Object { .. } => {
                    let mut object = JsObject::ordinary();
                    object.proto = Some(self.object_proto.clone());
                    JsValue::object(object)
                }
                SerializedNode::Array { .. } => {
                    let mut object = JsObject::array();
                    object.proto = Some(self.array_proto.clone());
                    JsValue::object(object)
                }
                SerializedNode::Boxed { kind, value } => {
                    let primitive = structured_atom_value(value, &objects)?;
                    let (prototype, slot) = match kind {
                        SerializedBoxKind::Boolean => (&self.boolean_proto, "[[BooleanData]]"),
                        SerializedBoxKind::Number => (&self.number_proto, "[[NumberData]]"),
                        SerializedBoxKind::BigInt => (&self.bigint_proto, "[[BigIntData]]"),
                        SerializedBoxKind::String => (&self.string_proto, "[[StringData]]"),
                    };
                    let mut object = JsObject::ordinary();
                    object.proto = Some(prototype.clone());
                    object.set_own(slot, primitive);
                    JsValue::object(object)
                }
                SerializedNode::Date(timestamp) => {
                    let mut object = JsObject::ordinary();
                    object.proto = Some(self.date_proto.clone());
                    object.set_own("[[DateValue]]", JsValue::Num(*timestamp));
                    JsValue::object(object)
                }
                SerializedNode::RegExp { pattern, flags } => self.make_regexp(Rc::new(
                    amazing_regex::Regex::new_utf16(pattern, flags).map_err(regexp_error)?,
                ))?,
                SerializedNode::ArrayBuffer {
                    bytes,
                    max_byte_length,
                    resizable,
                } => self.binary_new_buffer(bytes.clone(), *max_byte_length, *resizable, false),
                SerializedNode::ArrayBufferView { .. } => continue,
                SerializedNode::Map(_) => JsValue::Object(std_collection_map_new(self)),
                SerializedNode::Set(_) => JsValue::Object(std_collection_set_new(self)),
                SerializedNode::Error { name, message, .. } => {
                    let mut object = JsObject::ordinary();
                    object.exotic = Exotic::Error;
                    object.proto = self
                        .global_env
                        .resolve(name)
                        .and_then(|constructor| self.get_prop(&constructor, "prototype").ok())
                        .and_then(|value| {
                            if let JsValue::Object(proto) = value {
                                Some(proto)
                            } else {
                                None
                            }
                        })
                        .or_else(|| Some(self.object_proto.clone()));
                    object.set_own("name", JsValue::str(name.as_str()));
                    if let Some(message) = message {
                        object.set_own("message", JsValue::str_units(message.clone()));
                    }
                    JsValue::object(object)
                }
                SerializedNode::Host(value) => host.deserialize_host(value, self)?,
            };
            objects[index] = Some(value);
        }
        for (index, node) in serialized.nodes.iter().enumerate() {
            let SerializedNode::ArrayBufferView {
                buffer,
                byte_offset,
                byte_length,
                kind,
            } = node
            else {
                continue;
            };
            let Some(JsValue::Object(buffer)) = objects.get(*buffer).and_then(Clone::clone) else {
                return Err(self.structured_clone_error("invalid ArrayBuffer view reference"));
            };
            let kind = match kind {
                Some(name) => Some(
                    BinaryKind::from_name(name)
                        .ok_or_else(|| self.structured_clone_error("invalid TypedArray kind"))?,
                ),
                None => None,
            };
            objects[index] =
                Some(self.binary_new_view(buffer, *byte_offset, *byte_length, kind)?);
        }
        for (index, node) in serialized.nodes.iter().enumerate() {
            let value = objects[index].as_ref().unwrap().clone();
            match node {
                SerializedNode::Object { properties }
                | SerializedNode::Array { properties, .. } => {
                    let JsValue::Object(object) = &value else {
                        unreachable!()
                    };
                    for (key, atom) in properties {
                        let child = structured_atom_value(atom, &objects)?;
                        object
                            .borrow_mut()
                            .set_own(crate::value::JsString::from_units(key.clone()), child);
                    }
                    if let SerializedNode::Array { length, .. } = node {
                        object
                            .borrow_mut()
                            .define(
                                "length",
                                symbol_data(JsValue::num(*length as f64), true, false),
                            )
                            .unwrap();
                    }
                }
                SerializedNode::Map(entries) => {
                    let JsValue::Object(object) = &value else {
                        unreachable!()
                    };
                    for (key, child) in entries {
                        std_collection_map_put(
                            object,
                            structured_atom_value(key, &objects)?,
                            structured_atom_value(child, &objects)?,
                        );
                    }
                }
                SerializedNode::Set(entries) => {
                    let JsValue::Object(object) = &value else {
                        unreachable!()
                    };
                    for atom in entries {
                        let child = structured_atom_value(atom, &objects)?;
                        if let Exotic::Set(state) = &mut object.borrow_mut().exotic {
                            state.entries.push(std_collection_key(&child));
                        }
                    }
                }
                SerializedNode::Error {
                    cause: Some(cause), ..
                } => {
                    let JsValue::Object(object) = &value else {
                        unreachable!()
                    };
                    object
                        .borrow_mut()
                        .set_own("cause", structured_atom_value(cause, &objects)?);
                }
                _ => {}
            }
        }
        structured_atom_value(&serialized.root, &objects)
    }
}

fn native_structured_clone(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let value = args.first().cloned().unwrap_or(JsValue::Undefined);
    let options = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    let transfer = if options.is_undefined() || options.is_null() {
        Vec::new()
    } else if options.is_object() {
        let transfer = i.get_prop(&options, "transfer")?;
        if transfer.is_undefined() {
            Vec::new()
        } else {
            i.iterate(&transfer)?
        }
    } else {
        return Err(Throw::type_error(
            "structuredClone options must be an object",
        ));
    };
    let serialized = i.structured_serialize_with_transfer(&value, &transfer)?;
    i.structured_deserialize(&serialized)
}

#[cfg(test)]
mod structured_clone_tests {
    use super::*;

    #[test]
    fn bulk_structured_clone_table() {
        let cases = [
            ("structuredClone(undefined) === undefined", true),
            ("structuredClone(123n) === 123n", true),
            (
                "var a = { x: 1 }; var b = structuredClone(a); b !== a && b.x === 1",
                true,
            ),
            (
                "var a = {}; a.self = a; var b = structuredClone(a); b !== a && b.self === b",
                true,
            ),
            (
                "var a = {}; var b = structuredClone([a, a]); b[0] === b[1] && b[0] !== a",
                true,
            ),
            (
                "var a = []; a.length = 3; a[1] = 'x'; var b = structuredClone(a); b.length === 3 && b[1] === 'x' && !(0 in b)",
                true,
            ),
            (
                "var m = new Map(); m.set('self', m); var c = structuredClone(m); c !== m && c.get('self') === c",
                true,
            ),
            (
                "var s = new Set(); s.add(s); var c = structuredClone(s); c !== s && c.has(c)",
                true,
            ),
            (
                "var b = new ArrayBuffer(4); var a = new Uint8Array(b); a[0] = 17; var c = structuredClone(a); c !== a && c[0] === 17 && c.buffer !== b",
                true,
            ),
            (
                "var b = new ArrayBuffer(4); var a = new Uint8Array(b); a[0] = 9; var c = structuredClone({ b, a }); c.a.buffer === c.b && c.a[0] === 9",
                true,
            ),
            (
                "var b = new ArrayBuffer(4); var d = new DataView(b); d.setUint8(1, 31); var c = structuredClone({ b, d }); c.d.buffer === c.b && c.d.getUint8(1) === 31",
                true,
            ),
            (
                "var b = new ArrayBuffer(4); var c = structuredClone(b, { transfer: [b] }); b.byteLength === 0 && c.byteLength === 4",
                true,
            ),
            (
                "var b = new ArrayBuffer(4); var a = new Uint8Array(b); a[0] = 7; var c = structuredClone({ b, a }, { transfer: [b] }); b.byteLength === 0 && c.a.buffer === c.b && c.a[0] === 7",
                true,
            ),
            (
                "var n = structuredClone(new Number(5)); n instanceof Number && n.valueOf() === 5",
                true,
            ),
            (
                "var s = structuredClone(new String('abc')); s instanceof String && s.valueOf() === 'abc'",
                true,
            ),
            (
                "var b = structuredClone(new Boolean(true)); b instanceof Boolean && b.valueOf() === true",
                true,
            ),
            (
                "var b = structuredClone(Object(99n)); b.valueOf() === 99n",
                true,
            ),
            (
                "var d = structuredClone(new Date(42)); d instanceof Date && d['[[DateValue]]'] === 42",
                true,
            ),
            (
                "var r = structuredClone(/abc/gi); r !== null && r.source === 'abc' && r.flags === 'gi'",
                true,
            ),
            (
                "var e = structuredClone(new TypeError('bad')); e instanceof TypeError && e.message === 'bad'",
                true,
            ),
            (
                "var e = structuredClone(new EvalError('bad')); e instanceof EvalError && e.message === 'bad'",
                true,
            ),
            (
                "try { structuredClone(function() {}); false } catch (e) { e.name === 'DataCloneError' }",
                true,
            ),
            (
                "try { structuredClone(Symbol('x')); false } catch (e) { e.name === 'DataCloneError' }",
                true,
            ),
            (
                "var b = new ArrayBuffer(2); try { structuredClone(b, { transfer: [b, b] }); false } catch (e) { e.name === 'DataCloneError' && b.byteLength === 2 }",
                true,
            ),
            (
                "var b = new ArrayBuffer(2); try { structuredClone({}, { transfer: [b] }); false } catch (e) { e.name === 'DataCloneError' && b.byteLength === 2 }",
                true,
            ),
            (
                "var b = new ArrayBuffer(2); try { structuredClone({ b, fn: function() {} }, { transfer: [b] }); false } catch (e) { e.name === 'DataCloneError' && b.byteLength === 2 }",
                true,
            ),
        ];
        for (script, expected) in cases {
            let mut interpreter = Interpreter::new();
            let result = interpreter
                .run_script(script)
                .unwrap_or_else(|error| panic!("{script}: {error:?}"));
            assert_eq!(result, JsValue::Bool(expected), "{script}");
        }
    }

    #[test]
    fn wire_graph_is_send_and_realm_independent() {
        fn assert_send<T: Send>() {}
        assert_send::<SerializedValue>();
        let mut source = Interpreter::new();
        let value = source
            .run_script("var x = { value: 4 }; x.self = x; x")
            .unwrap();
        let wire = source.structured_serialize(&value).unwrap();
        let mut target = Interpreter::new();
        let restored = target.structured_deserialize(&wire).unwrap();
        assert_ne!(restored, value);
        let self_ref = target.get_prop(&restored, "self").unwrap();
        assert_eq!(self_ref, restored);
        assert_eq!(
            target.get_prop(&restored, "value").unwrap(),
            JsValue::num(4.0)
        );
    }
}
