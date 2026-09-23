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

use crate::value::{JsValue, Throw};

/// Shared handle to one environment frame. The only way code outside this
/// module touches an environment.
#[derive(Clone)]
pub struct EnvHandle(pub Rc<RefCell<Env>>);

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

    pub fn is_uninitialized(&self) -> bool {
        !self.target().0.borrow().initialized
    }

    pub fn same_target(&self, other: &BindingCell) -> bool {
        Rc::ptr_eq(&self.target().0, &other.target().0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvKind {
    /// Function body environment: owns `var` declarations.
    Function,
    /// Arrow body environment: owns locals but inherits the outer `this`.
    Arrow,
    /// Block / loop-body / catch environment.
    Block,
    /// The global environment: lookups missing here become ReferenceErrors.
    Global,
}

pub struct Env {
    pub kind: EnvKind,
    pub parent: Option<EnvHandle>,
    /// Per-binding cells: assignment mutates through `&self` without
    /// borrowing the whole frame, so a held `borrow()` on the frame never
    /// conflicts with an inner write.
    vars: HashMap<String, BindingCell>,
    /// Receiver recorded by a call (`None` = transparent frame).
    pub this_binding: Option<JsValue>,
    /// Derived constructors have a local `this` binding that throws until super().
    this_uninitialized: bool,
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
        EnvHandle(Rc::new(RefCell::new(Env {
            kind,
            parent,
            vars: HashMap::new(),
            this_binding: None,
            this_uninitialized: false,
        })))
    }

    /// `var` declaration: mutable, non-lexical, redeclaration-friendly.
    pub fn declare_var(&self, name: &str, value: JsValue) -> Result<(), Throw> {
        self.0.borrow_mut().declare_raw(name, value, true, false)
    }

    /// `let`/`const` declaration.
    pub fn declare_lexical(&self, name: &str, value: JsValue, mutable: bool) -> Result<(), Throw> {
        self.0.borrow_mut().declare_raw(name, value, mutable, true)
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
