// Portions ported from boa-dev/boa (core/engine/src/builtins/typed_array/object.rs)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT.
// Copyright (c) 2019 Jason Williams.
// Whole IsTypedArrayFixedLength predicate; actual view and backing-store slots
// distinguish resizable ArrayBuffer from fixed views of growable shared buffers.
fn binary_typed_array_fixed_length(object: &Obj) -> Option<bool> {
    let BinaryState::View(view) = binary_state(object)? else { return None; };
    view.kind?; // DataView keeps ordinary object integrity semantics.
    if view.length.is_none() { return Some(false); }
    let buffer = view.buffer.borrow();
    Some(!buffer.resizable || buffer.shared)
}
