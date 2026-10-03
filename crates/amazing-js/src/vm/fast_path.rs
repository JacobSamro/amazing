use super::{BinOp, Interpreter, Reference};
use crate::env::EnvHandle;
use crate::value::{Exotic, JsValue, PropertyKey, Throw};
use std::cell::RefCell;
use std::rc::{Rc, Weak};

/// A monomorphic own-property slot guarded by object identity and its layout.
/// Weak ownership ensures a long-lived VM frame does not retain an object.
pub(super) struct InlineCache {
    object: Weak<RefCell<crate::value::JsObject>>,
    shape: u64,
    slot: usize,
}

fn cached<'a>(cache: &'a Option<InlineCache>, object: &crate::value::Obj) -> Option<&'a InlineCache> {
    let cache = cache.as_ref()?;
    let borrowed = object.borrow();
    (cache.shape == borrowed.shape_version
        && borrowed.host_identity.is_none()
        && matches!(&borrowed.exotic, Exotic::Ordinary)
        && cache.object.upgrade().is_some_and(|target| Rc::ptr_eq(&target, &object.0)))
        .then_some(cache)
}

fn fill(cache: &mut Option<InlineCache>, object: &crate::value::Obj, key: &PropertyKey) {
    let borrowed = object.borrow();
    if !borrowed.vm_cacheable || borrowed.host_identity.is_some()
        || !matches!(&borrowed.exotic, Exotic::Ordinary)
        || ["[[DomNode]]", "[[DomStyleNode]]", "[[DomDatasetNode]]", "[[DomClassListNode]]"]
            .iter().any(|name| borrowed.own_prop(*name).is_some()) { return; }
    let Some(slot) = borrowed.own_slot_index(key) else { return; };
    if borrowed.own_data_slot(key, slot).is_some() {
        *cache = Some(InlineCache {
            object: Rc::downgrade(&object.0), shape: borrowed.shape_version, slot,
        });
    }
}

pub(super) fn get(
    interpreter: &mut Interpreter, receiver: &JsValue, key: PropertyKey,
    cache: &mut Option<InlineCache>,
) -> Result<JsValue, Throw> {
    if receiver.is_null() || receiver.is_undefined() {
        return Err(Throw::type_error("cannot read a property of null or undefined"));
    }
    if let JsValue::Object(object) = receiver {
        if let Some(slot) = cached(cache, object) {
            if let Some(property) = object.borrow().own_data_slot(&key, slot.slot) {
                return Ok(property.value.clone());
            }
        }
    }
    let value = interpreter.get_prop(receiver, &key)?;
    if let JsValue::Object(object) = receiver { fill(cache, object, &key); }
    Ok(value)
}

pub(super) fn set(
    interpreter: &mut Interpreter, receiver: &JsValue, key: PropertyKey,
    value: JsValue, env: &EnvHandle, cache: &mut Option<InlineCache>,
) -> Result<(), Throw> {
    if let JsValue::Object(object) = receiver {
        if let Some(slot) = cached(cache, object) {
            if object.borrow_mut().set_own_data_slot(&key, slot.slot, value.clone()) {
                return Ok(());
            }
        }
    }
    // Slow path: the interpreter's full OrdinarySet semantics, which honour
    // prototype setters, accessors, non-writable data properties, array exotic
    // length, and sloppy-mode failures. `set_prop` is the internal own-property
    // helper and is NOT equivalent to a JS assignment.
    interpreter.put_reference(Reference::Property(receiver.clone(), key.clone()), value, env)?;
    if let JsValue::Object(object) = receiver { fill(cache, object, &key); }
    Ok(())
}

/// Numeric arithmetic/comparisons need no ToPrimitive or allocation.
pub(super) fn binary(op: BinOp, left: &JsValue, right: &JsValue) -> Option<JsValue> {
    let (JsValue::Num(a), JsValue::Num(b)) = (left, right) else { return None };
    use BinOp::*;
    Some(match op {
        Add => JsValue::num(a + b),
        Sub => JsValue::num(a - b),
        Mul => JsValue::num(a * b),
        Div => JsValue::num(a / b),
        Rem => JsValue::num(a % b),
        Pow => JsValue::num(a.powf(*b)),
        Lt => JsValue::Bool(a < b),
        Gt => JsValue::Bool(a > b),
        Le => JsValue::Bool(a <= b),
        Ge => JsValue::Bool(a >= b),
        StrictEq | EqEq => JsValue::Bool(a == b),
        StrictNeq | NotEq => JsValue::Bool(a != b),
        _ => return None,
    })
}
