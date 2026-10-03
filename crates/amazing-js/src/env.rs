//! Lexical environments and closures (spec §9.1).
//!
//! Design decisions (locked):
//! - An environment is [`EnvHandle`] = `Rc<RefCell<Env>>` chained through
//!   `parent`. Closures capture the handle, so bindings stay live after
//!   their scope exits.
//! - All mutation goes through `RefCell` at the [`EnvHandle`] boundary;
//!   `Env` itself is plain data. Accessors clone what they need and drop
//!   the borrow before returning, so no interpreter path can double-borrow.
//! - `const` immutability is enforced at assignment time.
//! - Block environments are transparent for `this`: a call records its
//!   receiver in the function's environment frame, arrows/blocks leave
//!   [`Env::this_binding`] empty and resolution walks outward (§8.1.1).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::value::{JsValue, Obj, PrivateName, Property, Throw};

/// Shared handle to one environment frame. The only way code outside this
/// module touches an environment.
#[derive(Clone)]
pub struct EnvHandle(pub Rc<RefCell<Env>>, Option<Rc<str>>);

impl std::fmt::Debug for EnvHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let b = self.0.borrow();
        write!(f, "[env {:?}; {} bindings]", b.kind, b.vars.len())
    }
}

/// One declared binding.
#[derive(Debug, Clone)]
pub struct Binding {
    pub value: JsValue,
    /// `false` for `const`.
    pub mutable: bool,
    /// `true` for `let`/`const`: lexical bindings reject ANY same-frame
    /// redeclaration (§14.3.1), unlike `var`-over-`var`.
    pub lexical: bool,
    /// Module declarations are allocated before evaluation. Imports point at
    /// the same exporter cell, so reads always observe later writes.
    pub initialized: bool,
    pub import_target: Option<BindingCell>,
}

#[derive(Clone)]
pub struct BindingCell(Rc<RefCell<Binding>>);

impl std::fmt::Debug for BindingCell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "BindingCell({:p})", Rc::as_ptr(&self.0))
    }
}

impl BindingCell {
    pub fn new_value(value: JsValue) -> BindingCell {
        BindingCell(Rc::new(RefCell::new(Binding {
            value,
            mutable: false,
            lexical: true,
            initialized: true,
            import_target: None,
        })))
    }

    fn target(&self) -> BindingCell {
        let mut cell = self.clone();
        for _ in 0..1024 {
            let next = cell.0.borrow().import_target.clone();
            match next {
                Some(next) if !Rc::ptr_eq(&next.0, &cell.0) => cell = next,
                _ => break,
            }
        }
        cell
    }

    pub fn read(&self) -> Result<JsValue, Throw> {
        let cell = self.target();
        let binding = cell.0.borrow();
        if !binding.initialized {
            return Err(Throw::reference_error("module binding is uninitialized"));
        }
        Ok(binding.value.clone())
    }

    /// Write an initialized mutable formal cell retained by mapped arguments.
    pub(crate) fn write_mutable(&self, value: JsValue) -> Result<(), Throw> {
        let mut binding = self.0.borrow_mut();
        if !binding.initialized { return Err(Throw::reference_error("parameter is uninitialized")); }
        if !binding.mutable || binding.import_target.is_some() {
            return Err(Throw::type_error("binding is not mutable"));
        }
        binding.value = value;
        Ok(())
    }

    pub fn is_uninitialized(&self) -> bool {
        !self.target().0.borrow().initialized
    }

    pub fn same_target(&self, other: &BindingCell) -> bool {
        Rc::ptr_eq(&self.target().0, &other.target().0)
    }

    /// `true` for a `let`/`const`/class/catch-parameter binding, as opposed to a `var`- or
    /// function-declaration-hoisted one. Used by Annex B.3.3's block-function var sync (interp)
    /// to confirm a same-named outer cell really is the legacy `var`-style promotion (never a
    /// same-named lexical binding, which redeclaration rules would have already rejected).
    pub fn is_lexical(&self) -> bool {
        self.target().0.borrow().lexical
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvKind {
    /// Function body environment: owns `var` declarations.
    Function,
    /// Arrow body environment: owns locals but inherits the outer `this`.
    Arrow,
    /// Non-strict parameter-expression lexical environment. Eval vars belong
    /// to the outer callee variable environment, not this binding frame.
    Parameter,
    /// Body variable environment below a parameter-expression environment.
    /// Owns var/eval declarations and inherits this/super from the call frame.
    Var,
    /// Block / loop-body / catch environment.
    Block,
    /// The global environment: lookups missing here become ReferenceErrors.
    Global,
    /// Module environment: a strict top-level var scope.
    Module,
}

pub struct Env {
    realm: Option<u64>,
    global_object: Option<Obj>,
    pub kind: EnvKind,
    pub parent: Option<EnvHandle>,
    /// Per-binding cells: assignment mutates through `&self` without
    /// borrowing the whole frame, so a held `borrow()` on the frame never
    /// conflicts with an inner write.
    vars: HashMap<String, BindingCell>,
    /// Live binding object for an HTML handler (a with-environment record).
    pub(crate) binding_object: Option<Obj>,
    /// Receiver recorded by a call (`None` = transparent frame).
    pub this_binding: Option<JsValue>,
    /// Derived constructors have a local `this` binding that throws until super().
    this_uninitialized: bool,
    /// This class body's own `#name -> PrivateName` map (§9.2's private
    /// environment), set once by `eval_class_inner` on the frame every
    /// method/field-initializer/static-block of that class captures. `None`
    /// everywhere else; lookup walks outward through parents regardless, so
    /// a nested class's own names shadow an enclosing class's same-named one.
    class_private: Option<Rc<HashMap<String, PrivateName>>>,
}

impl Env {
    /// Declares in THIS frame. Same-frame redeclaration is legal ONLY for
    /// `var`-over-`var`; any lexical binding involved (existing or new)
    /// throws (§14.3).
    fn declare_raw(
        &mut self,
        name: &str,
        value: JsValue,
        mutable: bool,
        lexical: bool,
    ) -> Result<(), Throw> {
        if let Some(cell) = self.vars.get(name) {
            let mut existing = cell.0.borrow_mut();
            if !existing.initialized
                && existing.import_target.is_none()
                && lexical
                && existing.lexical
            {
                existing.value = value;
                existing.initialized = true;
                return Ok(());
            }
        }
        match self.vars.get(name).map(|b| b.0.borrow()) {
            Some(existing) if !existing.mutable || !mutable || lexical || existing.lexical => {
                return Err(Throw::syntax_error(format!(
                    "identifier '{name}' has already been declared"
                )));
            }
            _ => {}
        }
        if !lexical && self.kind == EnvKind::Global {
            if let Some(object) = &self.global_object {
                let mut object = object.borrow_mut();
                if let Some(mut property) = object.own_prop(name).cloned() {
                    if property.writable && property.getter.is_none() && property.setter.is_none() {
                        property.value = value.clone();
                        let _ = object.define(name, property);
                    }
                } else {
                    let mut property = Property::data(value.clone());
                    property.configurable = false;
                    let _ = object.define(name, property);
                }
            }
        }
        // Var/function redeclaration reuses its existing binding identity.
        // Mapped arguments and escaped references retain this precise cell.
        if let Some(cell) = self.vars.get(name) {
            let mut existing = cell.0.borrow_mut();
            existing.value = value;
            existing.initialized = true;
            return Ok(());
        }
        self.vars.insert(
            name.to_string(),
            BindingCell(Rc::new(RefCell::new(Binding {
                value,
                mutable,
                lexical,
                initialized: true,
                import_target: None,
            }))),
        );
        Ok(())
    }

    /// §9.1.2 GetIdentifierReference: walk the chain; None = unresolvable.
    fn resolve_raw(&self, name: &str) -> Option<JsValue> {
        if let Some(b) = self.vars.get(name) {
            if self.kind == EnvKind::Global && !b.0.borrow().lexical {
                if let Some(object) = &self.global_object {
                    return object.borrow().get_own(name);
                }
            }
            return b.read().ok();
        }
        let p = self.parent.as_ref()?;
        p.0.borrow().resolve_raw(name)
    }

    /// §9.1.5 SetMutableBinding: mutates the nearest declaring frame.
    /// `Ok(false)` when unresolved (caller decides global creation);
    /// `Err` for assignment to `const`.
    fn assign_raw(&self, name: &str, value: JsValue) -> Result<bool, Throw> {
        if let Some(b) = self.vars.get(name) {
            if b.0.borrow().import_target.is_some() {
                return Err(Throw::type_error(format!("assignment to import '{name}'")));
            }
            let mut b = b.0.borrow_mut();
            if !b.initialized {
                return Err(Throw::reference_error(format!("{name} is uninitialized")));
            }
            if !b.mutable {
                return Err(Throw::type_error(format!(
                    "assignment to constant variable '{name}'"
                )));
            }
            if self.kind == EnvKind::Global && !b.lexical {
                if let Some(object) = &self.global_object {
                    let mut object = object.borrow_mut();
                    if let Some(mut property) = object.own_prop(name).cloned() {
                        if property.writable && property.getter.is_none() && property.setter.is_none() {
                            property.value = value.clone();
                            let _ = object.define(name, property);
                        }
                    } else { object.set_own(name, value.clone()); }
                }
            }
            b.value = value;
            return Ok(true);
        }
        let p = match &self.parent {
            Some(p) => p,
            None => return Ok(false),
        };
        p.0.borrow().assign_raw(name, value)
    }

    /// Nearest recorded `this`, walking outward past transparent frames.
    /// Top level resolves to Undefined per §8.1.1.4 (sloppy global this).
    fn resolve_this_raw(&self) -> JsValue {
        if let Some(t) = &self.this_binding {
            return t.clone();
        }
        match &self.parent {
            Some(p) => p.0.borrow().resolve_this_raw(),
            None => JsValue::Undefined,
        }
    }

    fn resolve_this_checked_raw(&self) -> Result<JsValue, Throw> {
        if self.this_uninitialized {
            return Err(Throw::reference_error("this is uninitialized before super()"));
        }
        if let Some(this) = &self.this_binding {
            return Ok(this.clone());
        }
        match &self.parent {
            Some(parent) => parent.0.borrow().resolve_this_checked_raw(),
            None => Ok(JsValue::Undefined),
        }
    }
}

impl EnvHandle {
    pub fn new(kind: EnvKind, parent: Option<EnvHandle>) -> EnvHandle {
        let realm = parent.as_ref().and_then(EnvHandle::realm);
        let source_context = parent.as_ref().and_then(|parent| parent.1.clone());
        EnvHandle(Rc::new(RefCell::new(Env {
            realm,
            global_object: None,
            kind,
            parent,
            vars: HashMap::new(),
            binding_object: None,
            this_binding: None,
            this_uninitialized: false,
            class_private: None,
        })), source_context)
    }

    /// Shares live bindings while attaching an immutable originating-script locator.
    /// Captured handles retain this locator when another script uses the same globals.
    pub(crate) fn with_source_context(&self, referrer: &str) -> Self {
        Self(self.0.clone(), Some(Rc::from(referrer)))
    }

    pub(crate) fn source_context(&self) -> Option<&str> { self.1.as_deref() }

    pub(crate) fn new_object(object: Obj, parent: EnvHandle) -> EnvHandle {
        let frame = Self::new(EnvKind::Block, Some(parent));
        frame.0.borrow_mut().binding_object = Some(object);
        frame
    }

    /// Installs this frame's own class's private-name map (§9.2 NewPrivateEnvironment).
    /// Only `eval_class_inner` calls this, once per class, on the frame every
    /// element of that class captures as its defining environment.
    pub(crate) fn set_class_private(&self, names: HashMap<String, PrivateName>) {
        self.0.borrow_mut().class_private = Some(Rc::new(names));
    }

    /// Textual names visible to eval's early-error pass; runtime identity lookup
    /// still uses resolve_private and its nearest-environment shadowing.
    pub(crate) fn visible_private_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        let mut cursor = Some(self.clone());
        while let Some(env) = cursor {
            let frame = env.0.borrow();
            if let Some(private) = &frame.class_private { names.extend(private.keys().cloned()); }
            cursor = frame.parent.clone();
        }
        names.sort();
        names.dedup();
        names
    }

    /// Resolves a `#name` textual reference to its `PrivateName` identity by
    /// walking outward through enclosing class private environments, nearest
    /// first, so a nested class's own name shadows an outer one. `None`
    /// means no enclosing class declares that name (an unresolvable private
    /// reference — a SyntaxError at parse time per spec; the interpreter has
    /// no static pass, so callers throw at evaluation time instead).
    pub fn resolve_private(&self, name: &str) -> Option<PrivateName> {
        let frame = self.0.borrow();
        if let Some(map) = &frame.class_private {
            if let Some(found) = map.get(name) {
                return Some(found.clone());
            }
        }
        frame.parent.as_ref().and_then(|parent| parent.resolve_private(name))
    }

    pub(crate) fn set_global_object(&self, object: Obj) { self.0.borrow_mut().global_object = Some(object); }

    pub(crate) fn global_binding_object(&self, name: &str) -> Option<Obj> {
        let frame = self.0.borrow();
        if let Some(cell) = frame.vars.get(name) {
            return (frame.kind == EnvKind::Global && !cell.0.borrow().lexical)
                .then(|| frame.global_object.clone()).flatten();
        }
        frame.parent.as_ref().and_then(|parent| parent.global_binding_object(name))
    }

    pub(crate) fn realm(&self) -> Option<u64> { self.0.borrow().realm }
    pub(crate) fn set_realm(&self, realm: u64) { self.0.borrow_mut().realm = Some(realm); }

    /// Nearest realm's global object, walking outward past object/with scopes
    /// so a with-scope's final fallback checks the correct realm (§9.1.1.4.15).
    pub(crate) fn realm_global_object(&self) -> Option<Obj> {
        let frame = self.0.borrow();
        frame.global_object.clone()
            .or_else(|| frame.parent.as_ref().and_then(EnvHandle::realm_global_object))
    }


    /// `var` declaration: mutable, non-lexical, redeclaration-friendly.
    pub fn declare_var(&self, name: &str, value: JsValue) -> Result<(), Throw> {
        self.0.borrow_mut().declare_raw(name, value, true, false)
    }

    /// VarDeclarationInstantiation leaves an existing var/function value in
    /// place. An existing lexical binding is still a declaration conflict.
    pub fn hoist_var(&self, name: &str) -> Result<(), Throw> {
        let mut env = self.0.borrow_mut();
        if let Some(existing) = env.vars.get(name) {
            if existing.0.borrow().lexical {
                return Err(Throw::syntax_error(format!(
                    "identifier '{name}' has already been declared"
                )));
            }
            return Ok(());
        }
        let value = if env.kind == EnvKind::Global {
            env.global_object.as_ref().and_then(|object| object.borrow().get_own(name)).unwrap_or(JsValue::Undefined)
        } else { JsValue::Undefined };
        env.declare_raw(name, value, true, false)
    }

    pub(crate) fn has_local_lexical(&self, name: &str) -> bool {
        self.0.borrow().vars.get(name).is_some_and(|cell| cell.0.borrow().lexical)
    }

    /// Initialize a parameter's preallocated cell, preserving closure identity.
    /// Parameters permit body `var` declarations and sloppy duplicate names.
    pub(crate) fn initialize_parameter(&self, name: &str, value: JsValue) -> Result<(), Throw> {
        let env = self.0.borrow();
        let cell = env.vars.get(name)
            .ok_or_else(|| Throw::reference_error("parameter binding was not allocated"))?;
        let mut binding = cell.0.borrow_mut();
        binding.value = value;
        binding.initialized = true;
        Ok(())
    }

    /// Keep parameter names protected from eval var declarations while defaults
    /// are evaluated, then allow body var declarations to reuse their cells.
    pub(crate) fn finish_parameter_initialization(&self, name: &str) {
        if self.kind() == EnvKind::Parameter { return; }
        if let Some(cell) = self.0.borrow().vars.get(name) {
            cell.0.borrow_mut().lexical = false;
        }
    }

    /// `let`/`const` declaration.
    pub fn declare_lexical(&self, name: &str, value: JsValue, mutable: bool) -> Result<(), Throw> {
        self.0.borrow_mut().declare_raw(name, value, mutable, true)
    }

    /// True if `declare_lexical(name, ..)` would throw "already declared" on THIS frame right
    /// now, without actually declaring or mutating anything (mirrors `declare_raw`'s conflict
    /// rule with `lexical = true`: any existing same-frame binding conflicts, except the
    /// TDZ-placeholder case of an uninitialized lexical cell reserved for this same name).
    ///
    /// Used by a generator/async-generator call (JSVM-8, `start_thread_generator`) to detect a
    /// FunctionDeclarationInstantiation-time redeclaration conflict -- e.g. a parameter default's
    /// `eval("var x")` colliding with a top-level `let x` in the body -- eagerly, at call time,
    /// before deferring the body's own statement execution to a coroutine. Per spec (10.2.11 /
    /// 27.5.3), FDI itself always runs synchronously at call time, even for a generator; only
    /// the body's statements are suspended, so this conflict must still surface from the call
    /// itself rather than from the first `.next()`.
    pub fn would_conflict_as_lexical(&self, name: &str) -> bool {
        match self.0.borrow().vars.get(name) {
            None => false,
            Some(cell) => {
                let existing = cell.0.borrow();
                !(!existing.initialized && existing.import_target.is_none() && existing.lexical)
            }
        }
    }

    pub fn declare_uninitialized(&self, name: &str, mutable: bool) -> Result<BindingCell, Throw> {
        let mut env = self.0.borrow_mut();
        if env.vars.contains_key(name) {
            return Err(Throw::syntax_error(format!(
                "duplicate module binding '{name}'"
            )));
        }
        let cell = BindingCell(Rc::new(RefCell::new(Binding {
            value: JsValue::Undefined,
            mutable,
            lexical: true,
            initialized: false,
            import_target: None,
        })));
        env.vars.insert(name.to_owned(), cell.clone());
        Ok(cell)
    }

    pub fn declare_import(&self, name: &str, target: BindingCell) -> Result<(), Throw> {
        let mut env = self.0.borrow_mut();
        if env.vars.contains_key(name) {
            return Err(Throw::syntax_error(format!(
                "duplicate import binding '{name}'"
            )));
        }
        env.vars.insert(
            name.to_owned(),
            BindingCell(Rc::new(RefCell::new(Binding {
                value: JsValue::Undefined,
                mutable: false,
                lexical: true,
                initialized: true,
                import_target: Some(target),
            }))),
        );
        Ok(())
    }

    /// Remove the mirrored environment entry after deleting a configurable
    /// property of the global object. Lexical declarations are never removed.
    pub(crate) fn remove_object_binding(&self, name: &str) -> bool {
        let mut env = self.0.borrow_mut();
        if env.vars.get(name).is_some_and(|cell| cell.0.borrow().lexical) {
            return false;
        }
        env.vars.remove(name);
        true
    }

    /// The cell in this frame, without consulting parent environments.
    pub fn local_cell(&self, name: &str) -> Option<BindingCell> {
        self.0.borrow().vars.get(name).cloned()
    }

    pub fn is_uninitialized(&self, name: &str) -> bool {
        let frame = self.0.borrow();
        if let Some(cell) = frame.vars.get(name) {
            return cell.is_uninitialized();
        }
        frame
            .parent
            .as_ref()
            .is_some_and(|parent| parent.is_uninitialized(name))
    }

    pub fn resolve(&self, name: &str) -> Option<JsValue> {
        self.0.borrow().resolve_raw(name)
    }

    pub fn has(&self, name: &str) -> bool {
        self.0.borrow().vars.contains_key(name)
            || self
                .0
                .borrow()
                .parent
                .as_ref()
                .is_some_and(|parent| parent.has(name))
    }

    pub fn shares_binding_frame(&self, other: &EnvHandle, name: &str) -> bool {
        fn owner(env: &EnvHandle, name: &str) -> Option<EnvHandle> {
            let frame = env.0.borrow();
            if frame.vars.contains_key(name) { return Some(env.clone()); }
            let parent = frame.parent.clone();
            drop(frame);
            parent.as_ref().and_then(|parent| owner(parent, name))
        }
        match (owner(self, name), owner(other, name)) {
            (Some(left), Some(right)) => Rc::ptr_eq(&left.0, &right.0),
            _ => false,
        }
    }

    pub fn assign(&self, name: &str, value: JsValue) -> Result<bool, Throw> {
        self.0.borrow().assign_raw(name, value)
    }

    pub fn resolve_this(&self) -> JsValue {
        self.0.borrow().resolve_this_raw()
    }

    pub fn resolve_this_checked(&self) -> Result<JsValue, Throw> {
        self.0.borrow().resolve_this_checked_raw()
    }

    pub fn mark_this_uninitialized(&self) {
        let mut env = self.0.borrow_mut();
        env.this_binding = None;
        env.this_uninitialized = true;
    }

    pub fn initialize_this(&self, this: JsValue) -> Result<(), Throw> {
        let mut env = self.0.borrow_mut();
        if env.kind != EnvKind::Function {
            let parent = env.parent.clone().ok_or_else(|| {
                Throw::reference_error("super() used outside a derived constructor")
            })?;
            drop(env);
            return parent.initialize_this(this);
        }
        if !env.this_uninitialized {
            return Err(Throw::reference_error("super() has already initialized this"));
        }
        env.this_binding = Some(this);
        env.this_uninitialized = false;
        Ok(())
    }

    pub fn bind_this(&self, this: JsValue) {
        let mut env = self.0.borrow_mut();
        env.this_binding = Some(this);
        env.this_uninitialized = false;
    }

    pub fn kind(&self) -> EnvKind {
        self.0.borrow().kind
    }

    /// The realm global environment, independent of an active module/eval scope.
    pub(crate) fn global_scope(&self) -> EnvHandle {
        let mut current = self.clone();
        loop {
            let (kind, parent) = {
                let frame = current.0.borrow();
                (frame.kind, frame.parent.clone())
            };
            if kind == EnvKind::Global { return current; }
            match parent { Some(parent) => current = parent, None => return current }
        }
    }

    /// Variable declarations in sloppy direct eval target the nearest var environment.
    pub fn var_scope(&self) -> EnvHandle {
        let mut current = self.clone();
        loop {
            let (kind, parent) = {
                let env = current.0.borrow();
                (env.kind, env.parent.clone())
            };
            if matches!(kind, EnvKind::Global | EnvKind::Module | EnvKind::Function | EnvKind::Arrow | EnvKind::Var) {
                return current;
            }
            current = parent.expect("block environment has a parent");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> JsValue {
        JsValue::str(v)
    }

    #[test]
    fn declaration_resolution_and_shadowing() {
        let g = EnvHandle::new(EnvKind::Global, None);
        let f = EnvHandle::new(EnvKind::Function, Some(g.clone()));
        let b = EnvHandle::new(EnvKind::Block, Some(f.clone()));

        g.declare_var("x", s("global")).unwrap();
        f.declare_lexical("x", s("fn"), true).unwrap(); // shadows global
        b.declare_lexical("y", s("block"), true).unwrap();

        assert_eq!(b.resolve("x"), Some(s("fn"))); // innermost wins
        assert_eq!(b.resolve("y"), Some(s("block")));
        assert_eq!(f.resolve("y"), None); // block-scoped stays put
        assert!(b.has("x") && !f.has("y"));
    }

    #[test]
    fn const_blocks_assignment_and_any_redeclaration() {
        let g = EnvHandle::new(EnvKind::Global, None);
        g.declare_lexical("c", JsValue::num(1.0), false).unwrap();
        let err = g.assign("c", JsValue::num(2.0)).err().unwrap();
        assert!(err.describe().contains("constant"));
        // Same-scope redeclaration of ANY kind over a const throws (§14.3):
        // even `var c` is a SyntaxError when a lexical `c` exists here.
        assert!(g.declare_var("c", JsValue::num(3.0)).is_err());
        assert!(g.declare_lexical("c", JsValue::num(3.0), false).is_err());
        assert_eq!(g.resolve("c"), Some(JsValue::num(1.0)));
    }

    #[test]
    fn double_let_declaration_throws() {
        let g = EnvHandle::new(EnvKind::Global, None);
        g.declare_lexical("l", JsValue::Null, true).unwrap();
        assert!(g.declare_lexical("l", JsValue::Null, true).is_err());
        assert!(g.declare_var("l", JsValue::Null).is_err());
        // var-over-var is fine:
        g.declare_var("v", JsValue::Null).unwrap();
        g.declare_var("v", JsValue::num(1.0)).unwrap();
        assert_eq!(g.resolve("v"), Some(JsValue::num(1.0)));
    }

    #[test]
    fn unresolved_assignment_reports_false_for_global_creation() {
        let g = EnvHandle::new(EnvKind::Global, None);
        let f = EnvHandle::new(EnvKind::Function, Some(g.clone()));
        let hit = f.assign("implicit", JsValue::num(9.0)).unwrap();
        assert!(!hit); // interpreter turns this into a global write
    }

    #[test]
    fn this_walks_past_transparent_frames_to_nearest_function() {
        let recv = JsValue::object(crate::value::JsObject::ordinary());
        let func_env = EnvHandle::new(EnvKind::Function, None);
        func_env.bind_this(recv.clone());
        let block_env = EnvHandle::new(EnvKind::Block, Some(func_env.clone()));
        let arrow_env = EnvHandle::new(EnvKind::Function, Some(block_env));
        assert_eq!(arrow_env.resolve_this(), recv.clone());
        // Global top-level this is undefined (sloppy mode).
        assert_eq!(
            EnvHandle::new(EnvKind::Global, None).resolve_this(),
            JsValue::Undefined
        );
    }

    #[test]
    fn closures_see_later_writes_through_the_chain() {
        let shared = EnvHandle::new(EnvKind::Function, None);
        shared.declare_var("count", JsValue::num(0.0)).unwrap();
        let closure = shared.clone(); // what a function object captures
        shared.assign("count", JsValue::num(41.0)).unwrap();
        assert_eq!(closure.resolve("count"), Some(JsValue::num(41.0)));
    }

    #[test]
    fn deep_chain_resolution_does_not_reborrow_across_frames() {
        // Regression guard: resolve/assign walk parents via fresh borrows
        // per frame (no held borrow across frames), so deep chains work.
        let root = EnvHandle::new(EnvKind::Global, None);
        let mut cur = root.clone();
        root.declare_var("deep", s("found")).unwrap();
        for _ in 0..64 {
            cur = EnvHandle::new(EnvKind::Block, Some(cur));
        }
        assert_eq!(cur.resolve("deep"), Some(s("found")));
        assert!(cur.assign("deep", s("updated")).unwrap());
        assert_eq!(cur.resolve("deep"), Some(s("updated")));
    }

    #[test]
    fn import_alias_tracks_exporter_and_observes_tdz() {
        let exporter = EnvHandle::new(EnvKind::Global, None);
        let importer = EnvHandle::new(EnvKind::Global, None);
        let cell = exporter.declare_uninitialized("counter", true).unwrap();
        importer.declare_import("value", cell).unwrap();
        assert!(importer.is_uninitialized("value"));
        assert_eq!(importer.resolve("value"), None);
        exporter
            .declare_lexical("counter", JsValue::num(1.0), true)
            .unwrap();
        assert_eq!(importer.resolve("value"), Some(JsValue::num(1.0)));
        exporter.assign("counter", JsValue::num(2.0)).unwrap();
        assert_eq!(importer.resolve("value"), Some(JsValue::num(2.0)));
        assert!(importer.assign("value", JsValue::num(3.0)).is_err());
    }
}
