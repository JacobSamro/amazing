//! ElementInternals driver additions (HTML §4.13.7): setlike `@@iterator`.
use super::*;

impl RuntimeBindingHost<'_> {
    /// `HTMLCollection.prototype[Symbol.iterator]` is the realm's `Array.prototype.values`.
    pub(super) fn dom_define_array_values_iterator(&mut self, interface: Interface)
        -> Result<OptionalHookInstall, Throw> {
        let prototype = self.prototype(interface)
            .ok_or_else(|| Throw::type_error("Missing iterable prototype"))?;
        let global = self.js.binding_realm_global(self.js.binding_current_realm())?;
        let array = self.js.binding_get_property(&global, "Array")?;
        let array_prototype = self.js.binding_get_property(&array, "prototype")?;
        let values = self.js.binding_get_property(&array_prototype, "values")?;
        self.js.binding_define_symbol_data_property(&prototype,
            amazing_js::value::WellKnownSymbol::Iterator.symbol(), values,
            BindingPropertyFlags { writable: true, enumerable: false, configurable: true })?;
        Ok(OptionalHookInstall::Installed)
    }
    /// `CustomStateSet.prototype[Symbol.iterator]` is the same function as `values`.
    pub(super) fn dom_define_set_like_iterator(&mut self, interface: Interface)
        -> Result<OptionalHookInstall, Throw> {
        let prototype = self.prototype(interface)
            .ok_or_else(|| Throw::type_error("Missing setlike prototype"))?;
        let values = self.js.binding_get_property(&prototype, "values")?;
        self.js.binding_define_symbol_data_property(&prototype,
            amazing_js::value::WellKnownSymbol::Iterator.symbol(), values,
            BindingPropertyFlags { writable: true, enumerable: false, configurable: true })?;
        Ok(OptionalHookInstall::Installed)
    }
}

#[cfg(test)]
#[path = "runtime_host_html_internals_tests.rs"]
mod tests;
