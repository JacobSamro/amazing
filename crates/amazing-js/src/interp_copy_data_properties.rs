// Portions ported from lucid-softworks/lumen (crates/lumen/src/eval.rs,
// CopyDataProperties / copy_data_properties_into)
// @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
// Amazing uses actual PropertyKey identities, semantic exotic descriptors and
// UTF-16 primitive boxing instead of upstream encoded-string keys/Gc objects.
impl Interpreter {
    /// CopyDataProperties: callers supply a fresh ordinary literal/rest object.
    /// Each descriptor and value is read immediately before defining that key;
    /// excluded keys must not invoke an observable descriptor operation.
    fn copy_data_properties(&mut self, target: &JsValue, source: &JsValue,
        excluded: &[PropertyKey]) -> Result<(), Throw> {
        if source.is_null() || source.is_undefined() { return Ok(()); }
        let from = object_to_object(self, source.clone())?;
        let JsValue::Object(from_object) = &from else { unreachable!("ToObject result") };
        let JsValue::Object(target_object) = target else { unreachable!("fresh copy destination") };
        // OwnPropertyKeys snapshots the key list, while descriptors remain live.
        // Clone semantic results and release all borrows before invoking Get.
        let keys = self.own_property_keys(&from)?;
        for key in keys {
            if excluded.iter().any(|item| item == &key) { continue; }
            let descriptor = self.default_get_own_property(from_object, &key)?;
            if !descriptor.is_some_and(|property| property.enumerable) { continue; }
            let value = self.get_prop(&from, &key)?;
            // Destinations are ordinary newly created objects. Define data,
            // never Set: inherited setters and the legacy __proto__ setter
            // cannot observe copying, and existing literal accessors are reset.
            target_object.borrow_mut().define(&key, Property::data(value))
                .map_err(|_| Throw::type_error("could not create copied data property"))?;
        }
        Ok(())
    }
}
