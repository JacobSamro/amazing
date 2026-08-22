//! A small, panic-free tree-walking ECMAScript runtime.
//!
//! This is the Phase 6 execution milestone. It implements ordinary values,
//! objects, arrays, functions, closures, control flow, coercion and a small
//! set of ES built-ins using only `std`. Modules, generators, symbols and a
//! real asynchronous job queue remain explicit later milestones.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::ast::{self, Binding, ExprKind, ForInit, ForLeft, MemberProp, PropKey, StmtKind};
use crate::env::{EnvHandle, EnvKind};
use crate::value::{
    number_to_string, Exotic, FunctionBody, JsFunction, JsObject, JsValue, MapKey, MapState, Obj,
    PendingReaction, PromiseFate, PromiseState, Property, SetState, Throw,
};

pub const MAX_CALL_DEPTH: u32 = 512;

enum Flow {
    Normal,
    Break(Option<String>),
    Continue(Option<String>),
    Return(JsValue),
}

struct PromiseJob {
    reaction: PendingReaction,
    fate: PromiseFate,
}

struct TimerJob {
    id: u32,
    due: Instant,
    callback: JsValue,
    args: Vec<JsValue>,
}

struct CallFrame {
    this: JsValue,
    home: Option<Obj>,
}

#[derive(Clone)]
struct DomListener {
    node: u32,
    event: String,
    callback: JsValue,
    property: bool,
}

enum Reference {
    Binding(String),
    Property(JsValue, String),
}

/// Host hooks for the small browser-facing DOM surface.
///
/// The JS core deliberately knows nothing about the arena DOM types. A
/// higher-level driver supplies this trait and the interpreter exposes the
/// resulting live nodes as ordinary JavaScript objects.
pub trait DomHost {
    fn query_selector(&mut self, selector: &str) -> Option<u32>;
    fn get_text_content(&self, node: u32) -> Option<String>;
    fn set_text_content(&mut self, node: u32, text: &str) -> Result<(), String>;
    fn get_attribute(&self, node: u32, name: &str) -> Option<Option<String>>;
    fn set_attribute(&mut self, node: u32, name: &str, value: &str) -> Result<(), String>;
    fn create_element(&mut self, name: &str) -> Result<u32, String>;
    fn append_child(&mut self, parent: u32, child: u32) -> Result<(), String>;
    fn append_text(&mut self, parent: u32, text: &str) -> Result<(), String>;
    fn get_style(&self, node: u32, property: &str) -> Option<String>;
    fn set_style(&mut self, node: u32, property: &str, value: &str) -> Result<(), String>;
}

pub struct Interpreter {
    pub global_env: EnvHandle,
    pub global_obj: Obj,
    pub object_proto: Obj,
    pub function_proto: Obj,
    pub array_proto: Obj,
    pub map_proto: Obj,
    pub set_proto: Obj,
    pub date_proto: Obj,
    pub promise_proto: Obj,
    pub output: RefCell<Vec<String>>,
    depth: u32,
    native_stack: Vec<Obj>,
    jobs: VecDeque<PromiseJob>,
    completion: JsValue,
    frames: Vec<CallFrame>,
    dom_host: Option<Rc<RefCell<dyn DomHost>>>,
    dom_listeners: Vec<DomListener>,
    timers: Vec<TimerJob>,
    next_timer_id: u32,
}

impl Interpreter {
    pub fn new() -> Self {
        let object_proto = Obj(Rc::new(RefCell::new(JsObject::ordinary())));
        let function_proto = Obj(Rc::new(RefCell::new({
            let mut o = JsObject::ordinary();
            o.proto = Some(object_proto.clone());
            o
        })));
        let array_proto = Obj(Rc::new(RefCell::new({
            let mut o = JsObject::ordinary();
            o.proto = Some(object_proto.clone());
            o
        })));
        let map_proto = Obj(Rc::new(RefCell::new({
            let mut o = JsObject::ordinary();
            o.proto = Some(object_proto.clone());
            o
        })));
        let set_proto = Obj(Rc::new(RefCell::new({
            let mut o = JsObject::ordinary();
            o.proto = Some(object_proto.clone());
            o
        })));
        let date_proto = Obj(Rc::new(RefCell::new({
            let mut o = JsObject::ordinary();
            o.proto = Some(object_proto.clone());
            o
        })));
        let promise_proto = Obj(Rc::new(RefCell::new({
            let mut o = JsObject::ordinary();
            o.proto = Some(object_proto.clone());
            o
        })));
        let global_obj = Obj(Rc::new(RefCell::new({
            let mut o = JsObject::ordinary();
            o.proto = Some(object_proto.clone());
            o
        })));
        let global_env = EnvHandle::new(EnvKind::Global, None);
        let mut i = Self {
            global_env,
            global_obj,
            object_proto,
            function_proto,
            array_proto,
            map_proto,
            set_proto,
            date_proto,
            promise_proto,
            output: RefCell::new(Vec::new()),
            depth: 0,
            native_stack: Vec::new(),
            jobs: VecDeque::new(),
            completion: JsValue::Undefined,
            frames: Vec::new(),
            dom_host: None,
            dom_listeners: Vec::new(),
            timers: Vec::new(),
            next_timer_id: 1,
        };
        i.global_obj
            .borrow_mut()
            .set_own("globalThis", JsValue::Object(i.global_obj.clone()));
        i.install_builtins();
        i
    }

    /// Attach a live host and install the browser-facing `document` global.
    pub fn set_dom_host(&mut self, host: Rc<RefCell<dyn DomHost>>) {
        self.dom_host = Some(host);
        let document = self.make_dom_document();
        self.global_obj
            .borrow_mut()
            .set_own("document", document.clone());
        let _ = self.global_env.declare_lexical("document", document, false);
    }

    /// Runs promise reactions queued by a host callback. Browser integrations
    /// call this after dispatching an input event or timer task.
    pub fn drain_microtasks(&mut self) -> Result<(), Throw> {
        self.drain_jobs()
    }

    pub fn schedule_timer(
        &mut self,
        callback: JsValue,
        delay_ms: u64,
        args: Vec<JsValue>,
    ) -> u32 {
        let id = self.next_timer_id;
        self.next_timer_id = self.next_timer_id.wrapping_add(1).max(1);
        self.timers.push(TimerJob {
            id,
            due: Instant::now() + Duration::from_millis(delay_ms),
            callback,
            args,
        });
        id
    }

    pub fn cancel_timer(&mut self, id: u32) {
        self.timers.retain(|timer| timer.id != id);
    }

    /// Executes all timers whose deadline has passed, then drains resulting
    /// Promise reactions. Returns whether at least one timer ran.
    pub fn pump_timers(&mut self) -> Result<bool, Throw> {
        let now = Instant::now();
        let mut due = Vec::new();
        self.timers.retain(|timer| {
            if timer.due <= now {
                due.push((timer.callback.clone(), timer.args.clone()));
                false
            } else {
                true
            }
        });
        for (callback, args) in due.iter() {
            self.call_value(callback, JsValue::Undefined, args.clone())?;
            self.drain_jobs()?;
        }
        Ok(!due.is_empty())
    }

    /// Dispatch a browser-style event to listeners registered on a live DOM
    /// node. The driver uses this for its first interactive tracer; a later
    /// window shell can call the same API from native input handling.
    pub fn dispatch_dom_event(&mut self, node: u32, event_type: &str) -> Result<(), Throw> {
        let target = self.make_dom_element(node);
        let mut event_obj = JsObject::ordinary();
        event_obj.proto = Some(self.object_proto.clone());
        event_obj.set_own("type", JsValue::str(event_type));
        event_obj.set_own("target", target.clone());
        event_obj.set_own("currentTarget", target.clone());
        let event = JsValue::Object(Obj(Rc::new(RefCell::new(event_obj))));
        let callbacks: Vec<JsValue> = self
            .dom_listeners
            .iter()
            .filter(|listener| listener.node == node && listener.event == event_type)
            .map(|listener| listener.callback.clone())
            .collect();
        for callback in callbacks {
            self.call_value(&callback, target.clone(), vec![event.clone()])?;
        }
        self.drain_jobs()?;
        Ok(())
    }

    fn register_dom_listener(&mut self, node: u32, event: String, callback: JsValue) {
        self.dom_listeners.push(DomListener {
            node,
            event,
            callback,
            property: false,
        });
    }

    fn set_dom_handler(&mut self, node: u32, event: &str, callback: Option<JsValue>) {
        self.dom_listeners
            .retain(|listener| !(listener.node == node && listener.event == event && listener.property));
        if let Some(callback) = callback {
            self.dom_listeners.push(DomListener {
                node,
                event: event.to_owned(),
                callback,
                property: true,
            });
        }
    }

    fn dom_handler(&self, node: u32, event: &str) -> Option<JsValue> {
        self.dom_listeners
            .iter()
            .rev()
            .find(|listener| listener.node == node && listener.event == event && listener.property)
            .map(|listener| listener.callback.clone())
    }

    pub fn run_script(&mut self, source: &str) -> Result<JsValue, Throw> {
        let p = crate::parse_script(source).map_err(|e| Throw::syntax_error(e.to_string()))?;
        self.run_program(&p)
    }

    pub fn run_program(&mut self, p: &ast::Program) -> Result<JsValue, Throw> {
        let previous_completion = self.completion.clone();
        self.completion = JsValue::Undefined;
        let env = self.global_env.clone();
        self.hoist(&p.body, &env)?;
        for s in &p.body {
            if let StmtKind::Expression(e) = &s.kind {
                self.completion = self.eval_expr(e, &env)?;
                continue;
            }
            match self.exec_stmt(s, &env)? {
                Flow::Normal => {}
                Flow::Return(v) => self.completion = v,
                Flow::Break(_) | Flow::Continue(_) => {
                    return Err(Throw::syntax_error("illegal break or continue"))
                }
            }
        }
        self.drain_jobs()?;
        let result = self.completion.clone();
        self.completion = previous_completion;
        Ok(result)
    }

    fn hoist(&mut self, body: &[ast::Stmt], env: &EnvHandle) -> Result<(), Throw> {
        for s in body {
            match &s.kind {
                StmtKind::Var(d) if d.kind == ast::VarKind::Var => {
                    for x in &d.decls {
                        self.hoist_binding(&x.binding, env)?;
                    }
                }
                StmtKind::Function(f) => {
                    if let Some(n) = &f.name {
                        let v = self.make_function(f, env);
                        env.declare_var(&n.name, v)?;
                    }
                }
                StmtKind::Block(b) => self.hoist(&b.body, env)?,
                StmtKind::If { then, els, .. } => {
                    self.hoist(std::slice::from_ref(then), env)?;
                    if let Some(e) = els {
                        self.hoist(std::slice::from_ref(e), env)?;
                    }
                }
                StmtKind::While { body, .. } | StmtKind::DoWhile { body, .. } => {
                    self.hoist(std::slice::from_ref(body), env)?
                }
                StmtKind::For { init, body, .. } => {
                    if let Some(ForInit::Decl(d)) = init {
                        self.hoist_var(d, env)?;
                    }
                    self.hoist(std::slice::from_ref(body), env)?;
                }
                StmtKind::ForIn { left, body, .. } | StmtKind::ForOf { left, body, .. } => {
                    if let ForLeft::Decl(d) = left {
                        self.hoist_var(d, env)?;
                    }
                    self.hoist(std::slice::from_ref(body), env)?;
                }
                StmtKind::Labeled { body, .. } => self.hoist(std::slice::from_ref(body), env)?,
                StmtKind::Try(statement) => {
                    self.hoist(&statement.block.body, env)?;
                    if let Some(catch) = &statement.catch {
                        self.hoist(&catch.body.body, env)?;
                    }
                    if let Some(finally) = &statement.finally {
                        self.hoist(&finally.body, env)?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn hoist_var(&self, d: &ast::VarDecl, env: &EnvHandle) -> Result<(), Throw> {
        if d.kind == ast::VarKind::Var {
            for x in &d.decls {
                self.hoist_binding(&x.binding, env)?;
            }
        }
        Ok(())
    }

    fn hoist_binding(&self, binding: &Binding, env: &EnvHandle) -> Result<(), Throw> {
        match binding {
            Binding::Ident(identifier) => env.declare_var(&identifier.name, JsValue::Undefined)?,
            Binding::Array(elements) => {
                for element in elements.iter().flatten() {
                    self.hoist_binding(element, env)?;
                }
            }
            Binding::Object(properties) => {
                for (_, property) in properties {
                    self.hoist_binding(property, env)?;
                }
            }
            Binding::Rest(inner) | Binding::Default(inner, _) => self.hoist_binding(inner, env)?,
        }
        Ok(())
    }

    fn bind_pattern(
        &mut self,
        binding: &Binding,
        value: JsValue,
        kind: ast::VarKind,
        env: &EnvHandle,
    ) -> Result<(), Throw> {
        match binding {
            Binding::Ident(identifier) => match kind {
                ast::VarKind::Var => {
                    if !env.assign(&identifier.name, value.clone())? {
                        self.global_env.declare_var(&identifier.name, value)?;
                    }
                }
                ast::VarKind::Let => env.declare_lexical(&identifier.name, value, true)?,
                ast::VarKind::Const => env.declare_lexical(&identifier.name, value, false)?,
            },
            Binding::Default(inner, expression) => {
                let value = if value.is_undefined() {
                    self.eval_expr(expression, env)?
                } else {
                    value
                };
                self.bind_pattern(inner, value, kind, env)?;
            }
            Binding::Rest(inner) => self.bind_pattern(inner, value, kind, env)?,
            Binding::Array(elements) => {
                let items = self.iterate(&value).unwrap_or_default();
                let mut index = 0usize;
                for element in elements {
                    if let Some(element) = element {
                        if let Binding::Rest(rest) = element.as_ref() {
                            let rest_value = array_values(self, items[index..].to_vec())?;
                            self.bind_pattern(rest, rest_value, kind, env)?;
                            index = items.len();
                        } else {
                            let item = items.get(index).cloned().unwrap_or(JsValue::Undefined);
                            self.bind_pattern(element, item, kind, env)?;
                            index += 1;
                        }
                    } else {
                        index += 1;
                    }
                }
            }
            Binding::Object(properties) => {
                let source = match value {
                    JsValue::Object(_) | JsValue::Str(_) => value,
                    JsValue::Null | JsValue::Undefined => {
                        return Err(Throw::type_error("cannot destructure nullish value"))
                    }
                    other => other,
                };
                let mut used = Vec::new();
                for (key, property) in properties {
                    match key {
                        ast::BindingKey::Static(name) if name == "..." => {
                            let rest = self.object_rest(&source, &used)?;
                            self.bind_pattern(property, rest, kind, env)?;
                        }
                        ast::BindingKey::Static(name) => {
                            used.push(name.clone());
                            let item = self.get_prop(&source, name)?;
                            self.bind_pattern(property, item, kind, env)?;
                        }
                        ast::BindingKey::Computed(expression) => {
                            let computed_value = self.eval_expr(expression, env)?;
                            let computed = self.to_property_key(&computed_value)?;
                            used.push(computed.clone());
                            let item = self.get_prop(&source, &computed)?;
                            self.bind_pattern(property, item, kind, env)?;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn object_rest(&mut self, source: &JsValue, used: &[String]) -> Result<JsValue, Throw> {
        let rest = self.object_value();
        for key in self.enumerable_keys(source) {
            if !used.iter().any(|used_key| used_key == &key) {
                let value = self.get_prop(source, &key)?;
                self.set_prop(&rest, &key, value)?;
            }
        }
        Ok(rest)
    }

    fn exec_stmt(&mut self, s: &ast::Stmt, env: &EnvHandle) -> Result<Flow, Throw> {
        match &s.kind {
            StmtKind::Empty | StmtKind::Debugger => Ok(Flow::Normal),
            StmtKind::Expression(e) => {
                self.completion = self.eval_expr(e, env)?;
                Ok(Flow::Normal)
            }
            StmtKind::Var(d) => {
                for x in &d.decls {
                    let v = x
                        .init
                        .as_ref()
                        .map(|e| self.eval_expr(e, env))
                        .transpose()?
                        .unwrap_or(JsValue::Undefined);
                    if d.kind == ast::VarKind::Const && x.init.is_none() {
                        return Err(Throw::syntax_error("const requires an initializer"));
                    }
                    self.bind_pattern(&x.binding, v, d.kind, env)?;
                }
                Ok(Flow::Normal)
            }
            StmtKind::Function(_) => Ok(Flow::Normal),
            StmtKind::Class(c) => {
                if let Some(n) = &c.name {
                    let v = self.eval_class(c, env)?;
                    env.declare_lexical(&n.name, v, true)?;
                }
                Ok(Flow::Normal)
            }
            StmtKind::Block(b) => {
                self.completion = JsValue::Undefined;
                let e = EnvHandle::new(EnvKind::Block, Some(env.clone()));
                self.exec_stmts(&b.body, &e)
            }
            StmtKind::If { cond, then, els } => {
                self.completion = JsValue::Undefined;
                if self.eval_expr(cond, env)?.to_boolean() {
                    self.exec_stmt(then, env)
                } else if let Some(e) = els {
                    self.exec_stmt(e, env)
                } else {
                    Ok(Flow::Normal)
                }
            }
            StmtKind::While { test, body } => {
                self.completion = JsValue::Undefined;
                self.loop_while(test, body, env, None)
            }
            StmtKind::DoWhile { body, test } => {
                self.completion = JsValue::Undefined;
                self.loop_do_while(body, test, env, None)
            }
            StmtKind::For {
                init,
                test,
                update,
                body,
            } => {
                self.completion = JsValue::Undefined;
                self.loop_for(
                    init.as_ref(),
                    test.as_ref(),
                    update.as_ref(),
                    body,
                    env,
                    None,
                )
            }
            StmtKind::ForIn { left, right, body } => {
                self.completion = JsValue::Undefined;
                let v = self.eval_expr(right, env)?;
                for k in self.enumerable_keys(&v) {
                    let iteration_env = for_iteration_env(left, env);
                    self.bind_for(left, JsValue::str(k), &iteration_env)?;
                    match self.exec_stmt(body, &iteration_env)? {
                        Flow::Normal | Flow::Continue(None) => {}
                        Flow::Break(None) => break,
                        x => return Ok(x),
                    }
                }
                Ok(Flow::Normal)
            }
            StmtKind::ForOf { left, right, body } => {
                self.completion = JsValue::Undefined;
                let v = self.eval_expr(right, env)?;
                self.exec_for_of(left, &v, body, env)
            }
            StmtKind::Switch { disc, cases } => {
                self.completion = JsValue::Undefined;
                let d = self.eval_expr(disc, env)?;
                let mut start = None;
                let mut def = None;
                for (n, c) in cases.iter().enumerate() {
                    if let Some(t) = &c.test {
                        let tv = self.eval_expr(t, env)?;
                        if self.strict_eq(&d, &tv) {
                            start = Some(n);
                            break;
                        }
                    } else {
                        def = Some(n);
                    }
                }
                let mut i = start.or(def);
                while let Some(n) = i {
                    match self.exec_stmts(&cases[n].body, env)? {
                        Flow::Normal => {
                            i = (n + 1 < cases.len()).then_some(n + 1);
                        }
                        Flow::Break(_) => break,
                        x => return Ok(x),
                    }
                }
                Ok(Flow::Normal)
            }
            StmtKind::Break { label: l } => Ok(Flow::Break(l.as_ref().map(|x| x.name.clone()))),
            StmtKind::Continue { label: l } => {
                Ok(Flow::Continue(l.as_ref().map(|x| x.name.clone())))
            }
            StmtKind::Labeled { label, body } => match &body.kind {
                StmtKind::While { test, body } => {
                    self.loop_while(test, body, env, Some(&label.name))
                }
                StmtKind::DoWhile { body, test } => {
                    self.loop_do_while(body, test, env, Some(&label.name))
                }
                StmtKind::For {
                    init,
                    test,
                    update,
                    body,
                } => self.loop_for(
                    init.as_ref(),
                    test.as_ref(),
                    update.as_ref(),
                    body,
                    env,
                    Some(&label.name),
                ),
                _ => match self.exec_stmt(body, env)? {
                    Flow::Break(Some(x)) if x == label.name => Ok(Flow::Normal),
                    x => Ok(x),
                },
            },
            StmtKind::Return(e) => Ok(Flow::Return(
                e.as_ref()
                    .map(|x| self.eval_expr(x, env))
                    .transpose()?
                    .unwrap_or(JsValue::Undefined),
            )),
            StmtKind::Throw(e) => Err(Throw(self.eval_expr(e, env)?)),
            StmtKind::Try(t) => {
                self.completion = JsValue::Undefined;
                self.exec_try(t, env)
            }
        }
    }

    fn exec_stmts(&mut self, body: &[ast::Stmt], env: &EnvHandle) -> Result<Flow, Throw> {
        for s in body {
            match self.exec_stmt(s, env)? {
                Flow::Normal => {}
                x => return Ok(x),
            }
        }
        Ok(Flow::Normal)
    }

    fn drain_jobs(&mut self) -> Result<(), Throw> {
        while let Some(job) = self.jobs.pop_front() {
            run_promise_reaction(job.reaction, job.fate, self);
        }
        Ok(())
    }
    fn loop_while(
        &mut self,
        test: &ast::Expr,
        body: &ast::Stmt,
        env: &EnvHandle,
        label: Option<&str>,
    ) -> Result<Flow, Throw> {
        while self.eval_expr(test, env)?.to_boolean() {
            match self.exec_stmt(body, env)? {
                Flow::Normal | Flow::Continue(None) => {}
                Flow::Break(None) => break,
                Flow::Break(Some(name)) if label == Some(name.as_str()) => break,
                Flow::Continue(Some(name)) if label == Some(name.as_str()) => continue,
                x => return Ok(x),
            }
        }
        Ok(Flow::Normal)
    }

    fn loop_do_while(
        &mut self,
        body: &ast::Stmt,
        test: &ast::Expr,
        env: &EnvHandle,
        label: Option<&str>,
    ) -> Result<Flow, Throw> {
        loop {
            match self.exec_stmt(body, env)? {
                Flow::Break(None) => break,
                Flow::Break(Some(name)) if label == Some(name.as_str()) => break,
                Flow::Continue(None) | Flow::Normal => {}
                Flow::Continue(Some(name)) if label == Some(name.as_str()) => {}
                Flow::Continue(name) => return Ok(Flow::Continue(name)),
                Flow::Break(name) => return Ok(Flow::Break(name)),
                Flow::Return(value) => return Ok(Flow::Return(value)),
            }
            if !self.eval_expr(test, env)?.to_boolean() {
                break;
            }
        }
        Ok(Flow::Normal)
    }

    fn loop_for(
        &mut self,
        init: Option<&ForInit>,
        test: Option<&ast::Expr>,
        update: Option<&ast::Expr>,
        body: &ast::Stmt,
        env: &EnvHandle,
        label: Option<&str>,
    ) -> Result<Flow, Throw> {
        let loop_env = EnvHandle::new(EnvKind::Block, Some(env.clone()));
        if let Some(init) = init {
            match init {
                ForInit::Decl(decl) => {
                    let fake = ast::Stmt::new(StmtKind::Var((**decl).clone()), body.span.clone());
                    self.exec_stmt(&fake, &loop_env)?;
                }
                ForInit::Expr(expression) => {
                    self.eval_expr(expression, &loop_env)?;
                }
            }
        }
        let per_iteration_names = match init {
            Some(ForInit::Decl(decl)) if decl.kind != ast::VarKind::Var => decl
                .decls
                .iter()
                .flat_map(|declarator| binding_names(&declarator.binding))
                .collect(),
            _ => Vec::new(),
        };
        let state_env = if per_iteration_names.is_empty() {
            loop_env.clone()
        } else {
            let state = EnvHandle::new(EnvKind::Block, Some(env.clone()));
            for name in &per_iteration_names {
                let value = loop_env.resolve(name).unwrap_or(JsValue::Undefined);
                state.declare_lexical(name, value, true)?;
            }
            state
        };
        loop {
            let iteration_env = if per_iteration_names.is_empty() {
                loop_env.clone()
            } else {
                let iteration = EnvHandle::new(EnvKind::Block, Some(state_env.clone()));
                for name in &per_iteration_names {
                    let value = state_env.resolve(name).unwrap_or(JsValue::Undefined);
                    iteration.declare_lexical(name, value, true)?;
                }
                iteration
            };
            if let Some(test) = test {
                if !self.eval_expr(test, &iteration_env)?.to_boolean() {
                    break;
                }
            }
            match self.exec_stmt(body, &iteration_env)? {
                Flow::Break(None) => break,
                Flow::Break(Some(name)) if label == Some(name.as_str()) => break,
                Flow::Continue(None) => {}
                Flow::Continue(Some(name)) if label == Some(name.as_str()) => {}
                Flow::Normal => {}
                Flow::Continue(name) => return Ok(Flow::Continue(name)),
                Flow::Break(name) => return Ok(Flow::Break(name)),
                Flow::Return(value) => return Ok(Flow::Return(value)),
            }
            if per_iteration_names.is_empty() {
                if let Some(update) = update {
                    self.eval_expr(update, &loop_env)?;
                }
            } else {
                let update_env = EnvHandle::new(EnvKind::Block, Some(state_env.clone()));
                for name in &per_iteration_names {
                    let value = iteration_env.resolve(name).unwrap_or(JsValue::Undefined);
                    update_env.declare_lexical(name, value, true)?;
                }
                if let Some(update) = update {
                    self.eval_expr(update, &update_env)?;
                }
                for name in &per_iteration_names {
                    let value = update_env.resolve(name).unwrap_or(JsValue::Undefined);
                    state_env.assign(name, value)?;
                }
            }
        }
        Ok(Flow::Normal)
    }
    fn bind_for(&mut self, left: &ForLeft, value: JsValue, env: &EnvHandle) -> Result<(), Throw> {
        match left {
            ForLeft::Target(t) if is_destructuring_expr(t) => {
                self.assign_pattern_expr(t, &value, env)
            }
            ForLeft::Target(t) => self.assign_target(t, value, env),
            ForLeft::Decl(d) => self.bind_pattern(
                &d.decls
                    .first()
                    .ok_or_else(|| Throw::syntax_error("missing loop binding"))?
                    .binding,
                value,
                d.kind,
                env,
            ),
        }
    }
    fn exec_try(&mut self, t: &ast::TryStmt, env: &EnvHandle) -> Result<Flow, Throw> {
        let e = EnvHandle::new(EnvKind::Block, Some(env.clone()));
        self.hoist(&t.block.body, env)?;
        let mut out = match self.exec_stmts(&t.block.body, &e) {
            Ok(x) => Ok(x),
            Err(th) => {
                if let Some(c) = &t.catch {
                    let ce = EnvHandle::new(EnvKind::Block, Some(env.clone()));
                    if let Some(p) = &c.param {
                        ce.declare_lexical(&p.name, th.0, true)?;
                    }
                    self.hoist(&c.body.body, env)?;
                    self.exec_stmts(&c.body.body, &ce)
                } else {
                    Err(th)
                }
            }
        };
        if let Some(f) = &t.finally {
            let fe = EnvHandle::new(EnvKind::Block, Some(env.clone()));
            self.hoist(&f.body, env)?;
            match self.exec_stmts(&f.body, &fe) {
                Ok(Flow::Normal) => {}
                Ok(x) => out = Ok(x),
                Err(e) => out = Err(e),
            }
        }
        out
    }

    pub fn eval_expr(&mut self, e: &ast::Expr, env: &EnvHandle) -> Result<JsValue, Throw> {
        match &e.kind {
            ExprKind::Num(n) => self.numeric(n),
            ExprKind::Str(s) => Ok(JsValue::str(s.as_str())),
            ExprKind::Bool(b) => Ok(JsValue::Bool(*b)),
            ExprKind::Null => Ok(JsValue::Null),
            ExprKind::Ident(i) => env
                .resolve(&i.name)
                .ok_or_else(|| Throw::reference_error(format!("{} is not defined", i.name))),
            ExprKind::This => Ok(env.resolve_this()),
            ExprKind::Super => {
                let home = self
                    .frames
                    .last()
                    .and_then(|frame| frame.home.clone())
                    .ok_or_else(|| Throw::syntax_error("super used outside a method"))?;
                let prototype = home
                    .borrow()
                    .proto
                    .clone()
                    .ok_or_else(|| Throw::type_error("super prototype is null"))?;
                Ok(JsValue::Object(prototype))
            }
            ExprKind::Template(t) => {
                let mut s = String::new();
                for (i, q) in t.quasis.iter().enumerate() {
                    s.push_str(q);
                    if let Some(x) = t.exprs.get(i) {
                        let v = self.eval_expr(x, env)?;
                        s.push_str(&self.to_string(&v)?);
                    }
                }
                Ok(JsValue::str(s))
            }
            ExprKind::Array(xs) => {
                let a = JsValue::object({
                    let mut o = JsObject::array();
                    o.proto = Some(self.array_proto.clone());
                    o
                });
                let mut n = 0usize;
                for x in xs {
                    match x {
                        None => n += 1,
                        Some(x) if matches!(x.kind, ExprKind::Spread(_)) => {
                            if let ExprKind::Spread(y) = &x.kind {
                                let src = self.eval_expr(y, env)?;
                                let vals = self.iterate(&src)?;
                                for v in vals {
                                    self.set_prop(&a, &n.to_string(), v)?;
                                    n += 1;
                                }
                            }
                        }
                        Some(x) => {
                            let v = self.eval_expr(x, env)?;
                            self.set_prop(&a, &n.to_string(), v)?;
                            n += 1;
                        }
                    }
                }
                self.set_prop(&a, "length", JsValue::num(n as f64))?;
                Ok(a)
            }
            ExprKind::Object(xs) => {
                let o = self.object_value();
                for x in xs {
                    match x {
                        ast::ObjElem::Spread(y) => {
                            let src = self.eval_expr(y, env)?;
                            for k in self.enumerable_keys(&src) {
                                let v = self.get_prop(&src, &k)?;
                                self.set_prop(&o, &k, v)?;
                            }
                        }
                        ast::ObjElem::Prop(p) => {
                            let k = self.prop_key(&p.key, env)?;
                            let v = match &p.value {
                                ast::PropValue::Shorthand(i) => env
                                    .resolve(&i.name)
                                    .ok_or_else(|| Throw::reference_error(i.name.clone()))?,
                                ast::PropValue::Init(x) => self.eval_expr(x, env)?,
                                ast::PropValue::Method(m) => self.method_value(m, env),
                            };
                            self.set_prop(&o, &k, v)?;
                        }
                    }
                }
                Ok(o)
            }
            ExprKind::Spread(x) => self.eval_expr(x, env),
            ExprKind::Function(f) => Ok(self.make_function(f, env)),
            ExprKind::Arrow(a) => Ok(self.make_arrow(a, env)),
            ExprKind::Class(c) => self.eval_class(c, env),
            ExprKind::Paren(x) => self.eval_expr(x, env),
            ExprKind::Sequence(xs) => {
                let mut v = JsValue::Undefined;
                for x in xs {
                    v = self.eval_expr(x, env)?;
                }
                Ok(v)
            }
            ExprKind::Unary { op, operand } => self.unary(*op, operand, env),
            ExprKind::Update { op, prefix, target } => self.update(*op, *prefix, target, env),
            ExprKind::Binary { op, left, right } => {
                let a = self.eval_expr(left, env)?;
                let b = self.eval_expr(right, env)?;
                self.binary(*op, a, b)
            }
            ExprKind::Logical { op, left, right } => {
                let a = self.eval_expr(left, env)?;
                let take = match op {
                    ast::LogicalOp::And => a.to_boolean(),
                    ast::LogicalOp::Or => !a.to_boolean(),
                    ast::LogicalOp::Coalesce => a.is_null() || a.is_undefined(),
                };
                if take {
                    self.eval_expr(right, env)
                } else {
                    Ok(a)
                }
            }
            ExprKind::Assign { op, target, value } => self.assign(*op, target, value, env),
            ExprKind::Conditional {
                test,
                consequent,
                alternate,
            } => {
                if self.eval_expr(test, env)?.to_boolean() {
                    self.eval_expr(consequent, env)
                } else {
                    self.eval_expr(alternate, env)
                }
            }
            ExprKind::Call {
                callee,
                args,
                optional,
            } => self.call_expr(callee, args, *optional, env),
            ExprKind::New { callee, args } => {
                let f = self.eval_expr(callee, env)?;
                let a = self.eval_args(args, env)?;
                self.construct(&f, a)
            }
            ExprKind::Member {
                obj,
                prop,
                optional,
            } => {
                let o = self.eval_expr(obj, env)?;
                if *optional && (o.is_null() || o.is_undefined()) {
                    Ok(JsValue::Undefined)
                } else {
                    let k = self.member_key(prop, env)?;
                    self.get_prop(&o, &k)
                }
            }
            ExprKind::Yield { .. } => Err(Throw::type_error("generators are not implemented")),
        }
    }

    fn numeric(&self, n: &crate::tokens::Numeric) -> Result<JsValue, Throw> {
        if n.bigint {
            return Err(Throw::type_error("BigInt is not implemented"));
        }
        let raw: String = n.raw.chars().filter(|c| *c != '_').collect();
        let v = if n.radix == 10 {
            raw.parse().unwrap_or(f64::NAN)
        } else {
            u128::from_str_radix(&raw, n.radix as u32)
                .map(|x| x as f64)
                .unwrap_or(f64::NAN)
        };
        Ok(JsValue::num(v))
    }
    fn unary(&mut self, op: ast::UnOp, x: &ast::Expr, env: &EnvHandle) -> Result<JsValue, Throw> {
        match op {
            ast::UnOp::Typeof => {
                match &x.kind {
                    ExprKind::Ident(i) => {
                        return Ok(JsValue::str(
                            env.resolve(&i.name)
                                .map(|v| self.typeof_value(&v))
                                .unwrap_or("undefined"),
                        ));
                    }
                    ExprKind::Paren(inner) if matches!(inner.kind, ExprKind::Ident(_)) => {
                        if let ExprKind::Ident(i) = &inner.kind {
                            return Ok(JsValue::str(
                                env.resolve(&i.name)
                                    .map(|v| self.typeof_value(&v))
                                    .unwrap_or("undefined"),
                            ));
                        }
                    }
                    _ => {}
                }
                let v = self.eval_expr(x, env)?;
                Ok(JsValue::str(self.typeof_value(&v)))
            }
            ast::UnOp::Void => {
                self.eval_expr(x, env)?;
                Ok(JsValue::Undefined)
            }
            ast::UnOp::Delete => match &x.kind {
                ExprKind::Member { obj, prop, .. } => {
                    let o = self.eval_expr(obj, env)?;
                    let k = self.member_key(prop, env)?;
                    if let JsValue::Object(o) = o {
                        Ok(JsValue::Bool(o.borrow_mut().delete_own(&k)))
                    } else {
                        Ok(JsValue::Bool(true))
                    }
                }
                _ => {
                    self.eval_expr(x, env)?;
                    Ok(JsValue::Bool(true))
                }
            },
            ast::UnOp::Await => {
                let value = self.eval_expr(x, env)?;
                match value {
                    JsValue::Object(o) => {
                        let exotic = o.borrow().exotic.clone();
                        match exotic {
                            Exotic::Promise(PromiseState {
                                state: PromiseFate::Fulfilled(value),
                                ..
                            }) => Ok(value),
                            Exotic::Promise(PromiseState {
                                state: PromiseFate::Rejected(value),
                                ..
                            }) => Err(Throw(value)),
                            Exotic::Promise(PromiseState {
                                state: PromiseFate::Pending,
                                ..
                            }) => Ok(JsValue::Undefined),
                            _ => Ok(JsValue::Object(o)),
                        }
                    }
                    other => Ok(other),
                }
            }
            ast::UnOp::Not => Ok(JsValue::Bool(!self.eval_expr(x, env)?.to_boolean())),
            ast::UnOp::BitNot => {
                let v = self.eval_expr(x, env)?;
                Ok(JsValue::num((!self.to_int32(&v)?) as f64))
            }
            ast::UnOp::Neg => {
                let v = self.eval_expr(x, env)?;
                Ok(JsValue::num(-self.to_number(&v)?))
            }
            ast::UnOp::Pos => {
                let v = self.eval_expr(x, env)?;
                Ok(JsValue::num(self.to_number(&v)?))
            }
        }
    }
    fn update(
        &mut self,
        op: ast::UpdateOp,
        prefix: bool,
        x: &ast::Expr,
        env: &EnvHandle,
    ) -> Result<JsValue, Throw> {
        let (reference, old) = self.resolve_reference(x, env)?;
        let n = self.to_number(&old)? + if op == ast::UpdateOp::Inc { 1.0 } else { -1.0 };
        self.put_reference(reference, JsValue::num(n), env)?;
        Ok(if prefix { JsValue::num(n) } else { old })
    }
    fn binary(&mut self, op: ast::BinOp, a: JsValue, b: JsValue) -> Result<JsValue, Throw> {
        use ast::BinOp::*;
        match op {
            Add => {
                let x = self.primitive(a)?;
                let y = self.primitive(b)?;
                if x.is_str() || y.is_str() {
                    Ok(JsValue::str(format!(
                        "{}{}",
                        self.to_string(&x)?,
                        self.to_string(&y)?
                    )))
                } else {
                    Ok(JsValue::num(self.to_number(&x)? + self.to_number(&y)?))
                }
            }
            Sub => Ok(JsValue::num(self.to_number(&a)? - self.to_number(&b)?)),
            Mul => Ok(JsValue::num(self.to_number(&a)? * self.to_number(&b)?)),
            Div => Ok(JsValue::num(self.to_number(&a)? / self.to_number(&b)?)),
            Rem => Ok(JsValue::num(self.to_number(&a)? % self.to_number(&b)?)),
            Pow => Ok(JsValue::num(self.to_number(&a)?.powf(self.to_number(&b)?))),
            Lt | Gt | Le | Ge => {
                let x = self.primitive(a)?;
                let y = self.primitive(b)?;
                let c = if x.is_str() && y.is_str() {
                    x.as_str().unwrap().cmp(y.as_str().unwrap())
                } else {
                    self.to_number(&x)?
                        .partial_cmp(&self.to_number(&y)?)
                        .unwrap_or(std::cmp::Ordering::Equal)
                };
                Ok(JsValue::Bool(match op {
                    Lt => c.is_lt(),
                    Gt => c.is_gt(),
                    Le => c.is_le(),
                    Ge => c.is_ge(),
                    _ => false,
                }))
            }
            EqEq | NotEq => {
                let e = self.loose_eq(&a, &b)?;
                Ok(JsValue::Bool(if op == EqEq { e } else { !e }))
            }
            StrictEq | StrictNeq => {
                let e = self.strict_eq(&a, &b);
                Ok(JsValue::Bool(if op == StrictEq { e } else { !e }))
            }
            InstanceOf => self.instanceof(&a, &b),
            In => {
                let k = self.to_property_key(&a)?;
                match b {
                    JsValue::Object(o) => Ok(JsValue::Bool(o.borrow().has_property(&k))),
                    _ => Err(Throw::type_error("right operand of in is not an object")),
                }
            }
            Shl | Shr | UShr | BitAnd | BitOr | BitXor => {
                let x = self.to_int32(&a)?;
                let y = self.to_uint32(&b)? & 31;
                Ok(JsValue::num(match op {
                    Shl => (x << y) as f64,
                    Shr => (x >> y) as f64,
                    UShr => (x.wrapping_shr(y)) as u32 as f64,
                    BitAnd => (x & (y as i32)) as f64,
                    BitOr => (x | (y as i32)) as f64,
                    BitXor => (x ^ (y as i32)) as f64,
                    _ => 0.0,
                }))
            }
        }
    }
    fn assign(
        &mut self,
        op: ast::AssignOp,
        t: &ast::Expr,
        r: &ast::Expr,
        e: &EnvHandle,
    ) -> Result<JsValue, Throw> {
        if op == ast::AssignOp::Assign && is_destructuring_expr(t) {
            let value = self.eval_expr(r, e)?;
            self.assign_pattern_expr(t, &value, e)?;
            return Ok(value);
        }
        let (reference, old) = self.resolve_reference(t, e)?;
        if op == ast::AssignOp::Assign {
            let mut v = self.eval_expr(r, e)?;
            if let Some(name) = reference_name(&reference) {
                self.infer_function_name(&mut v, &name);
            }
            self.put_reference(reference, v.clone(), e)?;
            return Ok(v);
        }
        if matches!(&reference, Reference::Binding(name) if !e.has(name)) {
            return Err(Throw::reference_error(
                "assignment to an unresolved binding",
            ));
        }
        if op == ast::AssignOp::And && !old.to_boolean() {
            return Ok(old);
        }
        if op == ast::AssignOp::Or && old.to_boolean() {
            return Ok(old);
        }
        if op == ast::AssignOp::Nullish && !old.is_null() && !old.is_undefined() {
            return Ok(old);
        }
        let mut v = self.eval_expr(r, e)?;
        if let Some(name) = reference_name(&reference) {
            self.infer_function_name(&mut v, &name);
        }
        if matches!(
            op,
            ast::AssignOp::And | ast::AssignOp::Or | ast::AssignOp::Nullish
        ) {
            self.put_reference(reference, v.clone(), e)?;
            return Ok(v);
        }
        let bop = match op {
            ast::AssignOp::Add => ast::BinOp::Add,
            ast::AssignOp::Sub => ast::BinOp::Sub,
            ast::AssignOp::Mul => ast::BinOp::Mul,
            ast::AssignOp::Div => ast::BinOp::Div,
            ast::AssignOp::Rem => ast::BinOp::Rem,
            ast::AssignOp::Pow => ast::BinOp::Pow,
            ast::AssignOp::Shl => ast::BinOp::Shl,
            ast::AssignOp::Shr => ast::BinOp::Shr,
            ast::AssignOp::UShr => ast::BinOp::UShr,
            ast::AssignOp::BitAnd => ast::BinOp::BitAnd,
            ast::AssignOp::BitOr => ast::BinOp::BitOr,
            ast::AssignOp::BitXor => ast::BinOp::BitXor,
            _ => ast::BinOp::Add,
        };
        let out = self.binary(bop, old, v)?;
        self.put_reference(reference, out.clone(), e)?;
        Ok(out)
    }

    fn infer_function_name(&self, value: &mut JsValue, name: &str) {
        if let JsValue::Object(object) = value {
            let mut object = object.borrow_mut();
            if let Some(function) = &mut object.func {
                if function.name.is_empty() {
                    function.name = name.to_string();
                    object.set_own("name", JsValue::str(name));
                }
            }
        }
    }

    fn resolve_reference(
        &mut self,
        target: &ast::Expr,
        env: &EnvHandle,
    ) -> Result<(Reference, JsValue), Throw> {
        match &target.kind {
            ExprKind::Ident(identifier) => Ok((
                Reference::Binding(identifier.name.clone()),
                env.resolve(&identifier.name).unwrap_or(JsValue::Undefined),
            )),
            ExprKind::Member { obj, prop, .. } => {
                let base = self.eval_expr(obj, env)?;
                let key = match prop {
                    MemberProp::Ident(identifier) => identifier.name.clone(),
                    MemberProp::Computed(expression) => {
                        let property_value = self.eval_expr(expression, env)?;
                        if base.is_null() || base.is_undefined() {
                            return Err(Throw::type_error(
                                "cannot access a property on nullish value",
                            ));
                        }
                        self.to_property_key(&property_value)?
                    }
                };
                if base.is_null() || base.is_undefined() {
                    return Err(Throw::type_error(
                        "cannot access a property on nullish value",
                    ));
                }
                let value = self.get_prop(&base, &key)?;
                Ok((Reference::Property(base, key), value))
            }
            ExprKind::Paren(inner) => self.resolve_reference(inner, env),
            _ => Err(Throw::reference_error("invalid assignment target")),
        }
    }

    fn put_reference(
        &mut self,
        reference: Reference,
        value: JsValue,
        env: &EnvHandle,
    ) -> Result<(), Throw> {
        match reference {
            Reference::Binding(name) => {
                if !env.assign(&name, value.clone())? {
                    self.global_env.declare_var(&name, value)?;
                }
                Ok(())
            }
            Reference::Property(base, key) => self.set_prop(&base, &key, value),
        }
    }
    fn assign_target(&mut self, t: &ast::Expr, v: JsValue, e: &EnvHandle) -> Result<(), Throw> {
        let (reference, _) = self.resolve_reference(t, e)?;
        self.put_reference(reference, v, e)
    }

    fn assign_pattern_expr(
        &mut self,
        target: &ast::Expr,
        value: &JsValue,
        env: &EnvHandle,
    ) -> Result<(), Throw> {
        match &target.kind {
            ExprKind::Paren(inner) => self.assign_pattern_expr(inner, value, env),
            ExprKind::Assign {
                op: ast::AssignOp::Assign,
                target,
                value: default,
            } => {
                if value.is_undefined() {
                    let fallback = self.eval_expr(default, env)?;
                    self.assign_pattern_expr(target, &fallback, env)
                } else {
                    self.assign_pattern_expr(target, value, env)
                }
            }
            ExprKind::Array(elements) => {
                let items = self.iterate(value)?;
                let mut index = 0usize;
                for element in elements.iter().flatten() {
                    if let ExprKind::Spread(inner) = &element.kind {
                        let rest = array_values(self, items[index..].to_vec())?;
                        self.assign_pattern_expr(inner, &rest, env)?;
                        index = items.len();
                    } else {
                        let item = items.get(index).cloned().unwrap_or(JsValue::Undefined);
                        self.assign_pattern_expr(element, &item, env)?;
                        index += 1;
                    }
                }
                Ok(())
            }
            ExprKind::Object(elements) => {
                if value.is_null() || value.is_undefined() {
                    return Err(Throw::type_error("cannot destructure nullish value"));
                }
                let mut used = Vec::new();
                for element in elements {
                    match element {
                        ast::ObjElem::Spread(inner) => {
                            let rest = self.object_rest(value, &used)?;
                            self.assign_pattern_expr(inner, &rest, env)?;
                        }
                        ast::ObjElem::Prop(property) => {
                            let key = self.prop_key(&property.key, env)?;
                            used.push(key.clone());
                            let item = self.get_prop(value, &key)?;
                            match &property.value {
                                ast::PropValue::Shorthand(identifier) => {
                                    let reference = ast::Expr::new(
                                        ExprKind::Ident(identifier.clone()),
                                        identifier.span,
                                    );
                                    self.assign_pattern_expr(&reference, &item, env)?;
                                }
                                ast::PropValue::Init(expression) => {
                                    self.assign_pattern_expr(expression, &item, env)?;
                                }
                                ast::PropValue::Method(_) => {
                                    return Err(Throw::reference_error(
                                        "method is not an assignment target",
                                    ));
                                }
                            }
                        }
                    }
                }
                Ok(())
            }
            _ => self.assign_target(target, value.clone(), env),
        }
    }
    fn call_expr(
        &mut self,
        c: &ast::Expr,
        args: &[ast::Expr],
        optional: bool,
        e: &EnvHandle,
    ) -> Result<JsValue, Throw> {
        let (f, this) = if matches!(&c.kind, ExprKind::Super) {
            let home = self
                .frames
                .last()
                .and_then(|frame| frame.home.clone())
                .ok_or_else(|| Throw::syntax_error("super() used outside a method"))?;
            let super_prototype = home
                .borrow()
                .proto
                .clone()
                .ok_or_else(|| Throw::type_error("super constructor has no prototype"))?;
            let constructor = self.get_prop(&JsValue::Object(super_prototype), "constructor")?;
            let receiver = self
                .frames
                .last()
                .map(|frame| frame.this.clone())
                .unwrap_or(JsValue::Undefined);
            (constructor, receiver)
        } else if let ExprKind::Member {
            obj,
            prop,
            optional: m,
        } = &c.kind
        {
            let o = self.eval_expr(obj, e)?;
            if (optional || *m) && (o.is_null() || o.is_undefined()) {
                return Ok(JsValue::Undefined);
            }
            let k = self.member_key(prop, e)?;
            let receiver = if matches!(&obj.kind, ExprKind::Super) {
                self.frames
                    .last()
                    .map(|frame| frame.this.clone())
                    .unwrap_or(JsValue::Undefined)
            } else {
                o.clone()
            };
            (self.get_prop(&o, &k)?, receiver)
        } else {
            (self.eval_expr(c, e)?, JsValue::Undefined)
        };
        if optional && (f.is_null() || f.is_undefined()) {
            return Ok(JsValue::Undefined);
        }
        let argv = self.eval_args(args, e)?;
        self.call_value(&f, this, argv)
    }
    fn eval_args(&mut self, xs: &[ast::Expr], e: &EnvHandle) -> Result<Vec<JsValue>, Throw> {
        let mut out = Vec::new();
        for x in xs {
            if let ExprKind::Spread(y) = &x.kind {
                let v = self.eval_expr(y, e)?;
                out.extend(self.iterate(&v)?)
            } else {
                out.push(self.eval_expr(x, e)?);
            }
        }
        Ok(out)
    }
    fn member_key(&mut self, p: &MemberProp, e: &EnvHandle) -> Result<String, Throw> {
        match p {
            MemberProp::Ident(i) => Ok(i.name.clone()),
            MemberProp::Computed(x) => {
                let v = self.eval_expr(x, e)?;
                self.to_property_key(&v)
            }
        }
    }
    fn prop_key(&mut self, p: &PropKey, e: &EnvHandle) -> Result<String, Throw> {
        match p {
            PropKey::Ident(i) => Ok(i.name.clone()),
            PropKey::Str(s) => Ok(s.clone()),
            PropKey::Num(n) => Ok(number_to_string(self.numeric(n)?.to_number()?)),
            PropKey::Computed(x) => {
                let v = self.eval_expr(x, e)?;
                self.to_property_key(&v)
            }
        }
    }

    pub fn call_value(
        &mut self,
        f: &JsValue,
        this: JsValue,
        args: Vec<JsValue>,
    ) -> Result<JsValue, Throw> {
        let o = match f {
            JsValue::Object(o) => o.clone(),
            _ => return Err(Throw::type_error("value is not callable")),
        };
        let fun = o
            .borrow()
            .func
            .clone()
            .ok_or_else(|| Throw::type_error("value is not callable"))?;
        if self.depth >= MAX_CALL_DEPTH {
            return Err(Throw::range_error("maximum call stack size exceeded"));
        }
        self.depth += 1;
        self.frames.push(CallFrame {
            this: this.clone(),
            home: fun.home.clone(),
        });
        let previous_completion = self.completion.clone();
        self.completion = JsValue::Undefined;
        let r = (|| -> Result<JsValue, Throw> {
            match fun.body {
                FunctionBody::Native(n) => {
                    self.native_stack.push(o.clone());
                    let result = n(self, this, args);
                    self.native_stack.pop();
                    result
                }
                FunctionBody::Expr(x) => {
                    let e = EnvHandle::new(EnvKind::Function, Some(fun.env.clone()));
                    e.bind_this(if fun.is_arrow {
                        fun.env.resolve_this()
                    } else {
                        this
                    });
                    self.bind_params(&e, &fun.params, args)?;
                    self.eval_expr(&x, &e)
                }
                FunctionBody::Block(b) => {
                    let e = EnvHandle::new(EnvKind::Function, Some(fun.env.clone()));
                    e.bind_this(if fun.is_arrow {
                        fun.env.resolve_this()
                    } else {
                        this
                    });
                    self.bind_params(&e, &fun.params, args)?;
                    self.hoist(&b.body, &e)?;
                    match self.exec_stmts(&b.body, &e)? {
                        Flow::Return(v) => Ok(v),
                        _ => Ok(JsValue::Undefined),
                    }
                }
            }
        })();
        self.completion = previous_completion;
        self.frames.pop();
        self.depth -= 1;
        if fun.is_async {
            match r {
                Ok(value) => Ok(new_promise(self, PromiseFate::Fulfilled(value))),
                Err(Throw(value)) => Ok(new_promise(self, PromiseFate::Rejected(value))),
            }
        } else {
            r
        }
    }
    fn bind_params(
        &mut self,
        e: &EnvHandle,
        ps: &[ast::Param],
        args: Vec<JsValue>,
    ) -> Result<(), Throw> {
        for (i, p) in ps.iter().enumerate() {
            let v = if let Some(v) = args.get(i) {
                v.clone()
            } else if let Some(d) = &p.default {
                self.eval_expr(d, e)?
            } else {
                JsValue::Undefined
            };
            self.bind_pattern(&p.binding, v, ast::VarKind::Let, e)?;
        }
        let a = JsValue::object(JsObject::array());
        for (i, v) in args.into_iter().enumerate() {
            self.set_prop(&a, &i.to_string(), v)?;
        }
        self.set_prop(&a, "length", JsValue::num(ps.len() as f64))?;
        e.declare_var("arguments", a)?;
        Ok(())
    }
    fn construct(&mut self, f: &JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
        let o = match f {
            JsValue::Object(o) => o.clone(),
            _ => return Err(Throw::type_error("constructor is not an object")),
        };
        let fun = o
            .borrow()
            .func
            .clone()
            .ok_or_else(|| Throw::type_error("not a constructor"))?;
        if fun.is_arrow {
            return Err(Throw::type_error("arrow is not a constructor"));
        }
        let proto = match o.borrow().get_own("prototype") {
            Some(JsValue::Object(p)) => p,
            _ => self.object_proto.clone(),
        };
        let obj = Obj(Rc::new(RefCell::new({
            let mut x = JsObject::ordinary();
            x.proto = Some(proto);
            x
        })));
        let v = self.call_value(f, JsValue::Object(obj.clone()), args)?;
        if v.is_object() {
            Ok(v)
        } else {
            Ok(JsValue::Object(obj))
        }
    }
    fn make_function(&self, f: &ast::FunctionItem, e: &EnvHandle) -> JsValue {
        let mut o = JsObject::function(JsFunction {
            name: f.name.as_ref().map(|x| x.name.clone()).unwrap_or_default(),
            params: f.params.clone(),
            body: FunctionBody::Block(f.body.clone()),
            is_arrow: false,
            is_async: f.is_async,
            is_generator: f.is_generator,
            env: e.clone(),
            home: None,
            proto_for_new: None,
        });
        o.proto = Some(self.function_proto.clone());
        let h = Obj(Rc::new(RefCell::new(o)));
        let p = Obj(Rc::new(RefCell::new({
            let mut x = JsObject::ordinary();
            x.proto = Some(self.object_proto.clone());
            x
        })));
        h.borrow_mut()
            .set_own("prototype", JsValue::Object(p.clone()));
        h.borrow_mut().set_own(
            "name",
            JsValue::str(
                f.name
                    .as_ref()
                    .map(|name| name.name.clone())
                    .unwrap_or_default(),
            ),
        );
        h.borrow_mut().func.as_mut().unwrap().proto_for_new = Some(p);
        JsValue::Object(h)
    }
    fn make_arrow(&self, a: &ast::ArrowFunc, e: &EnvHandle) -> JsValue {
        let body = match &a.body {
            ast::ArrowBody::Expr(x) => FunctionBody::Expr((**x).clone()),
            ast::ArrowBody::Block(b) => FunctionBody::Block(b.clone()),
        };
        let mut o = JsObject::function(JsFunction {
            name: "".into(),
            params: a.params.clone(),
            body,
            is_arrow: true,
            is_async: a.is_async,
            is_generator: false,
            env: e.clone(),
            home: None,
            proto_for_new: None,
        });
        o.proto = Some(self.function_proto.clone());
        let handle = Obj(Rc::new(RefCell::new(o)));
        handle.borrow_mut().set_own("name", JsValue::str(""));
        JsValue::Object(handle)
    }
    fn method_value(&self, m: &ast::ClassMethod, e: &EnvHandle) -> JsValue {
        let mut o = JsObject::function(JsFunction {
            name: "".into(),
            params: m.params.clone(),
            body: FunctionBody::Block(m.body.clone()),
            is_arrow: false,
            is_async: m.is_async,
            is_generator: m.is_generator,
            env: e.clone(),
            home: None,
            proto_for_new: None,
        });
        o.proto = Some(self.function_proto.clone());
        let handle = Obj(Rc::new(RefCell::new(o)));
        handle.borrow_mut().set_own("name", JsValue::str(""));
        JsValue::Object(handle)
    }
    fn eval_class(&mut self, c: &ast::ClassItem, e: &EnvHandle) -> Result<JsValue, Throw> {
        let ctor = c.members.iter().find_map(|m| {
            if let ast::ClassMember::Method(x) = m {
                (x.kind == ast::MethodKind::Constructor).then_some(x)
            } else {
                None
            }
        });
        let f = if let Some(x) = ctor {
            self.method_value(x, e)
        } else {
            self.make_function(
                &ast::FunctionItem {
                    name: c.name.clone(),
                    params: Vec::new(),
                    body: ast::Block {
                        body: Vec::new(),
                        span: crate::tokens::Span::new(
                            crate::tokens::SourceLocation::new(1, 1),
                            crate::tokens::SourceLocation::new(1, 1),
                        ),
                    },
                    is_async: false,
                    is_generator: false,
                },
                e,
            )
        };
        if let JsValue::Object(o) = &f {
            if o.borrow().get_own("prototype").is_none() {
                let prototype = Obj(Rc::new(RefCell::new({
                    let mut object = JsObject::ordinary();
                    object.proto = Some(self.object_proto.clone());
                    object
                })));
                o.borrow_mut()
                    .set_own("prototype", JsValue::Object(prototype.clone()));
                o.borrow_mut().func.as_mut().unwrap().proto_for_new = Some(prototype);
            }
            let prototype = o.borrow().get_own("prototype");
            if let Some(JsValue::Object(p)) = prototype {
                p.borrow_mut().set_own("constructor", f.clone());
                if let Some(super_class) = &c.super_class {
                    let super_value = self.eval_expr(super_class, e)?;
                    let super_prototype = match super_value {
                        JsValue::Object(super_constructor) => match super_constructor
                            .borrow()
                            .get_own("prototype")
                        {
                            Some(JsValue::Object(prototype)) => prototype,
                            _ => {
                                return Err(Throw::type_error("super constructor has no prototype"))
                            }
                        },
                        _ => return Err(Throw::type_error("super class is not a constructor")),
                    };
                    p.borrow_mut().proto = Some(super_prototype);
                }
                let mut class_function = o.borrow_mut();
                if let Some(function) = &mut class_function.func {
                    function.home = Some(p.clone());
                }
                drop(class_function);
                for m in &c.members {
                    if let ast::ClassMember::Method(x) = m {
                        if x.kind != ast::MethodKind::Constructor {
                            let k = self.prop_key(&x.key, e)?;
                            let method = self.method_value(x, e);
                            if let JsValue::Object(method_object) = &method {
                                method_object.borrow_mut().func.as_mut().unwrap().home =
                                    Some(p.clone());
                            }
                            if matches!(x.kind, ast::MethodKind::Get | ast::MethodKind::Set) {
                                let function = match method {
                                    JsValue::Object(method_object) => {
                                        method_object.borrow().func.clone().unwrap()
                                    }
                                    _ => unreachable!(),
                                };
                                let mut property =
                                    p.borrow().own_prop(&k).cloned().unwrap_or(Property {
                                        value: JsValue::Undefined,
                                        writable: false,
                                        enumerable: false,
                                        configurable: true,
                                        getter: None,
                                        setter: None,
                                    });
                                if x.kind == ast::MethodKind::Get {
                                    property.getter = Some(Box::new(function));
                                } else {
                                    property.setter = Some(Box::new(function));
                                }
                                p.borrow_mut().define(&k, property).map_err(|_| {
                                    Throw::type_error("could not define class accessor")
                                })?;
                            } else {
                                self.set_prop(&JsValue::Object(p.clone()), &k, method)?;
                            }
                        }
                    }
                }
            }
        }
        Ok(f)
    }

    fn object_value(&self) -> JsValue {
        let mut o = JsObject::ordinary();
        o.proto = Some(self.object_proto.clone());
        JsValue::object(o)
    }

    fn make_dom_document(&self) -> JsValue {
        let mut document = JsObject::ordinary();
        document.proto = Some(self.object_proto.clone());
        document.set_own("[[DomNode]]", JsValue::num(0.0));
        document.set_own(
            "querySelector",
            Self::native("querySelector", native_dom_query_selector, &self.function_proto),
        );
        document.set_own(
            "createElement",
            Self::native("createElement", native_dom_create_element, &self.function_proto),
        );
        document.set_own(
            "addEventListener",
            Self::native("addEventListener", native_dom_add_event_listener, &self.function_proto),
        );
        document.set_own(
            "removeEventListener",
            Self::native("removeEventListener", native_dom_remove_event_listener, &self.function_proto),
        );
        JsValue::Object(Obj(Rc::new(RefCell::new(document))))
    }

    fn make_dom_element(&self, id: u32) -> JsValue {
        let mut element = JsObject::ordinary();
        element.proto = Some(self.object_proto.clone());
        element.set_own("[[DomNode]]", JsValue::num(id as f64));
        let getter = Self::native("get textContent", native_dom_text_get, &self.function_proto);
        let setter = Self::native("set textContent", native_dom_text_set, &self.function_proto);
        let getter = match getter {
            JsValue::Object(o) => o.borrow().func.clone().expect("native getter"),
            _ => unreachable!(),
        };
        let setter = match setter {
            JsValue::Object(o) => o.borrow().func.clone().expect("native setter"),
            _ => unreachable!(),
        };
        let _ = element.define(
            "textContent",
            Property {
                value: JsValue::Undefined,
                writable: false,
                enumerable: true,
                configurable: true,
                getter: Some(Box::new(getter)),
                setter: Some(Box::new(setter)),
            },
        );
        element.set_own(
            "getAttribute",
            Self::native("getAttribute", native_dom_get_attribute, &self.function_proto),
        );
        element.set_own(
            "setAttribute",
            Self::native("setAttribute", native_dom_set_attribute, &self.function_proto),
        );
        element.set_own(
            "append",
            Self::native("append", native_dom_append, &self.function_proto),
        );
        element.set_own(
            "addEventListener",
            Self::native("addEventListener", native_dom_add_event_listener, &self.function_proto),
        );
        element.set_own(
            "removeEventListener",
            Self::native("removeEventListener", native_dom_remove_event_listener, &self.function_proto),
        );
        element.set_own(
            "click",
            Self::native("click", native_dom_click, &self.function_proto),
        );
        let mut style = JsObject::ordinary();
        style.proto = Some(self.object_proto.clone());
        style.set_own("[[DomStyleNode]]", JsValue::num(id as f64));
        element.set_own("style", JsValue::Object(Obj(Rc::new(RefCell::new(style)))));
        let onclick_getter = Self::native("get onclick", native_dom_onclick_get, &self.function_proto);
        let onclick_setter = Self::native("set onclick", native_dom_onclick_set, &self.function_proto);
        let onclick_getter = match onclick_getter {
            JsValue::Object(o) => o.borrow().func.clone().expect("native onclick getter"),
            _ => unreachable!(),
        };
        let onclick_setter = match onclick_setter {
            JsValue::Object(o) => o.borrow().func.clone().expect("native onclick setter"),
            _ => unreachable!(),
        };
        let _ = element.define(
            "onclick",
            Property {
                value: JsValue::Null,
                writable: false,
                enumerable: true,
                configurable: true,
                getter: Some(Box::new(onclick_getter)),
                setter: Some(Box::new(onclick_setter)),
            },
        );
        JsValue::Object(Obj(Rc::new(RefCell::new(element))))
    }

    fn dom_style_id(object: &Obj) -> Option<u32> {
        match object.borrow().get_own("[[DomStyleNode]]") {
            Some(JsValue::Num(n)) if n.is_finite() && n >= 0.0 && n.fract() == 0.0 => {
                Some(n as u32)
            }
            _ => None,
        }
    }

    fn get_prop(&mut self, v: &JsValue, k: &str) -> Result<JsValue, Throw> {
        match v {
            JsValue::Object(o) => {
                if let Some(node) = Self::dom_style_id(o) {
                    if let Some(value) = dom_host(self)?.borrow().get_style(node, k) {
                        return Ok(JsValue::str(value));
                    }
                }
                if k == "size" {
                    let size = match &o.borrow().exotic {
                        Exotic::Map(state) => Some(state.entries.len()),
                        Exotic::Set(state) => Some(state.entries.len()),
                        _ => None,
                    };
                    if let Some(size) = size {
                        return Ok(JsValue::num(size as f64));
                    }
                }
                if let Some(property) = o.borrow().own_prop(k).cloned() {
                    if let Some(getter) = property.getter {
                        let function = JsValue::Object(Obj(Rc::new(RefCell::new(
                            JsObject::function(*getter),
                        ))));
                        self.call_value(&function, JsValue::Object(o.clone()), Vec::new())
                    } else {
                        Ok(property.value)
                    }
                } else if let Some(p) = o.borrow().proto.clone() {
                    self.get_prop(&JsValue::Object(p), k)
                } else {
                    Ok(JsValue::Undefined)
                }
            }
            JsValue::Str(s) => {
                if k == "split" {
                    Ok(Self::native(
                        "split",
                        native_string_split,
                        &self.function_proto,
                    ))
                } else if k == "trim" {
                    Ok(Self::native(
                        "trim",
                        native_string_trim,
                        &self.function_proto,
                    ))
                } else if k == "charAt" {
                    Ok(Self::native(
                        "charAt",
                        native_string_char_at,
                        &self.function_proto,
                    ))
                } else if k == "indexOf" {
                    Ok(Self::native(
                        "indexOf",
                        native_string_index_of,
                        &self.function_proto,
                    ))
                } else if k == "includes" {
                    Ok(Self::native(
                        "includes",
                        native_string_includes,
                        &self.function_proto,
                    ))
                } else if k == "replace" {
                    Ok(Self::native(
                        "replace",
                        native_string_replace,
                        &self.function_proto,
                    ))
                } else if k == "startsWith" {
                    Ok(Self::native(
                        "startsWith",
                        native_string_starts_with,
                        &self.function_proto,
                    ))
                } else if k == "endsWith" {
                    Ok(Self::native(
                        "endsWith",
                        native_string_ends_with,
                        &self.function_proto,
                    ))
                } else if k == "length" {
                    Ok(JsValue::num(s.chars().count() as f64))
                } else if let Ok(n) = k.parse::<usize>() {
                    Ok(s.chars()
                        .nth(n)
                        .map(|c| JsValue::str(c.to_string()))
                        .unwrap_or(JsValue::Undefined))
                } else {
                    Ok(JsValue::Undefined)
                }
            }
            _ => Ok(JsValue::Undefined),
        }
    }
    fn set_prop(&mut self, v: &JsValue, k: &str, x: JsValue) -> Result<(), Throw> {
        let o = match v {
            JsValue::Object(o) => o.clone(),
            _ => return Err(Throw::type_error("cannot set property on primitive")),
        };
        if let Some(node) = Self::dom_style_id(&o) {
            let value = self.to_string(&x)?;
            dom_host(self)?
                .borrow_mut()
                .set_style(node, k, &value)
                .map_err(Throw::type_error)?;
            return Ok(());
        }
        if let Some(property) = o.borrow().own_prop(k).cloned() {
            if let Some(setter) = property.setter {
                let function =
                    JsValue::Object(Obj(Rc::new(RefCell::new(JsObject::function(*setter)))));
                self.call_value(&function, JsValue::Object(o.clone()), vec![x])?;
                return Ok(());
            }
            if !property.writable {
                return Err(Throw::type_error("cannot assign to a read-only property"));
            }
            if property.getter.is_some() {
                return Err(Throw::type_error("property has no setter"));
            }
        } else if !o.borrow().extensible {
            return Err(Throw::type_error("object is not extensible"));
        }
        if k == "length" && matches!(o.borrow().exotic, Exotic::Array) {
            let n = self.to_number(&x)?;
            if !n.is_finite() || n < 0.0 || n.fract() != 0.0 {
                return Err(Throw::range_error("invalid array length"));
            }
        }
        o.borrow_mut().set_own(k, x);
        if let Ok(n) = k.parse::<usize>() {
            if matches!(o.borrow().exotic, Exotic::Array) {
                let len = match o.borrow().get_own("length") {
                    Some(JsValue::Num(n)) => n,
                    _ => 0.0,
                };
                if n as f64 + 1.0 > len {
                    o.borrow_mut()
                        .set_own("length", JsValue::num(n as f64 + 1.0));
                }
            }
        }
        Ok(())
    }
    fn enumerable_keys(&self, v: &JsValue) -> Vec<String> {
        match v {
            JsValue::Object(o) => o.borrow().own_keys_enumerable(),
            JsValue::Str(s) => (0..s.chars().count()).map(|i| i.to_string()).collect(),
            _ => Vec::new(),
        }
    }
    fn array_length(&self, v: &JsValue) -> usize {
        match v {
            JsValue::Object(o) => match o.borrow().get_own("length") {
                Some(JsValue::Num(n)) if n >= 0.0 => n as usize,
                _ => 0,
            },
            _ => 0,
        }
    }
    fn iterate(&mut self, v: &JsValue) -> Result<Vec<JsValue>, Throw> {
        if let JsValue::Object(object) = v {
            if matches!(object.borrow().exotic, Exotic::Array) {
                let n = match object.borrow().get_own("length") {
                    Some(JsValue::Num(n)) => n as usize,
                    _ => 0,
                };
                return Ok((0..n)
                    .map(|n| self.get_prop(v, &n.to_string()))
                    .collect::<Result<_, _>>()?);
            }
            let iterator_method = self.get_prop(v, "@@iterator")?;
            if callable_function(&iterator_method).is_some() {
                let iterator = self.call_value(&iterator_method, v.clone(), Vec::new())?;
                let same = matches!((v, &iterator), (JsValue::Object(left), JsValue::Object(right)) if Rc::ptr_eq(&left.0, &right.0));
                if !same {
                    return self.iterate(&iterator);
                }
            }
            let next = self.get_prop(v, "next")?;
            if !callable_function(&next).is_some() {
                return Err(Throw::type_error("value is not iterable"));
            }
            let mut values = Vec::new();
            for _ in 0..1_000_000usize {
                let result = match self.call_value(&next, v.clone(), Vec::new()) {
                    Ok(result) => result,
                    Err(error) => {
                        self.close_iterator(v);
                        return Err(error);
                    }
                };
                let result_object = match result {
                    JsValue::Object(result) => result,
                    _ => {
                        self.close_iterator(v);
                        return Err(Throw::type_error("iterator result is not an object"));
                    }
                };
                let done = result_object
                    .borrow()
                    .get_own("done")
                    .map(|value| value.to_boolean())
                    .unwrap_or(false);
                if done {
                    return Ok(values);
                }
                values.push(
                    result_object
                        .borrow()
                        .get_own("value")
                        .unwrap_or(JsValue::Undefined),
                );
            }
            self.close_iterator(v);
            return Err(Throw::range_error("iterator exceeded the iteration limit"));
        }
        match v {
            JsValue::Str(s) => Ok(s.chars().map(|c| JsValue::str(c.to_string())).collect()),
            _ => Err(Throw::type_error("value is not iterable")),
        }
    }
    fn close_iterator(&mut self, iterator: &JsValue) {
        let Ok(return_method) = self.get_prop(iterator, "return") else {
            return;
        };
        if callable_function(&return_method).is_some() {
            let _ = self.call_value(&return_method, iterator.clone(), Vec::new());
        }
    }

    fn exec_for_of(
        &mut self,
        left: &ForLeft,
        iterable: &JsValue,
        body: &ast::Stmt,
        env: &EnvHandle,
    ) -> Result<Flow, Throw> {
        let custom_iterator = self.custom_iterator(iterable)?;
        let Some(iterator) = custom_iterator else {
            for value in self.iterate(iterable)? {
                let iteration_env = for_iteration_env(left, env);
                self.bind_for(left, value, &iteration_env)?;
                match self.exec_stmt(body, &iteration_env)? {
                    Flow::Normal | Flow::Continue(None) => {}
                    Flow::Break(None) => break,
                    flow => return Ok(flow),
                }
            }
            return Ok(Flow::Normal);
        };
        loop {
            let next = self.get_prop(&iterator, "next")?;
            let result = match self.call_value(&next, iterator.clone(), Vec::new()) {
                Ok(result) => result,
                Err(error) => {
                    self.close_iterator(&iterator);
                    return Err(error);
                }
            };
            let result_object = match result {
                JsValue::Object(object) => object,
                _ => {
                    self.close_iterator(&iterator);
                    return Err(Throw::type_error("iterator result is not an object"));
                }
            };
            if result_object
                .borrow()
                .get_own("done")
                .map(|value| value.to_boolean())
                .unwrap_or(false)
            {
                break;
            }
            let value = result_object
                .borrow()
                .get_own("value")
                .unwrap_or(JsValue::Undefined);
            let iteration_env = for_iteration_env(left, env);
            self.bind_for(left, value, &iteration_env)?;
            match self.exec_stmt(body, &iteration_env) {
                Ok(Flow::Normal | Flow::Continue(None)) => {}
                Ok(Flow::Break(None)) => {
                    self.close_iterator(&iterator);
                    break;
                }
                Ok(flow) => {
                    self.close_iterator(&iterator);
                    return Ok(flow);
                }
                Err(error) => {
                    self.close_iterator(&iterator);
                    return Err(error);
                }
            }
        }
        Ok(Flow::Normal)
    }

    fn custom_iterator(&mut self, value: &JsValue) -> Result<Option<JsValue>, Throw> {
        let JsValue::Object(object) = value else {
            return Ok(None);
        };
        if matches!(object.borrow().exotic, Exotic::Array) {
            return Ok(None);
        }
        let method = self.get_prop(value, "@@iterator")?;
        if callable_function(&method).is_some() {
            let iterator = self.call_value(&method, value.clone(), Vec::new())?;
            let next = self.get_prop(&iterator, "next")?;
            if callable_function(&next).is_none() {
                return Err(Throw::type_error(
                    "iterator method did not return an iterator",
                ));
            }
            return Ok(Some(iterator));
        }
        let next = self.get_prop(value, "next")?;
        if callable_function(&next).is_some() {
            Ok(Some(value.clone()))
        } else {
            Ok(None)
        }
    }
    fn typeof_value(&self, v: &JsValue) -> &'static str {
        match v {
            JsValue::Undefined => "undefined",
            JsValue::Null => "object",
            JsValue::Bool(_) => "boolean",
            JsValue::Num(_) => "number",
            JsValue::Str(_) => "string",
            JsValue::Object(o) => {
                if o.borrow().func.is_some() {
                    "function"
                } else {
                    "object"
                }
            }
        }
    }
    fn primitive(&mut self, v: JsValue) -> Result<JsValue, Throw> {
        if !v.is_object() {
            return Ok(v);
        };
        if let JsValue::Object(object) = &v {
            for slot in ["[[BooleanData]]", "[[NumberData]]", "[[StringData]]"] {
                if let Some(value) = object.borrow().get_own(slot) {
                    return Ok(value);
                }
            }
        }
        for n in ["valueOf", "toString"] {
            let f = self.get_prop(&v, n)?;
            if matches!(f,JsValue::Object(ref o)if o.borrow().func.is_some()) {
                let x = self.call_value(&f, v.clone(), Vec::new())?;
                if !x.is_object() {
                    return Ok(x);
                }
            }
        }
        Err(Throw::type_error("cannot convert object to primitive"))
    }
    fn to_number(&mut self, v: &JsValue) -> Result<f64, Throw> {
        if v.is_object() {
            let p = self.primitive(v.clone())?;
            self.to_number(&p)
        } else {
            v.to_number()
        }
    }
    fn to_string(&mut self, v: &JsValue) -> Result<String, Throw> {
        if v.is_object() {
            let p = self.primitive(v.clone())?;
            self.to_string(&p)
        } else {
            Ok(crate::value::to_display_string(v))
        }
    }
    fn to_property_key(&mut self, v: &JsValue) -> Result<String, Throw> {
        self.to_string(v)
    }
    fn to_int32(&mut self, v: &JsValue) -> Result<i32, Throw> {
        let n = self.to_number(v)?;
        if !n.is_finite() || n == 0.0 {
            Ok(0)
        } else {
            Ok(n.trunc().rem_euclid(4294967296.0) as u32 as i32)
        }
    }
    fn to_uint32(&mut self, v: &JsValue) -> Result<u32, Throw> {
        Ok(self.to_int32(v)? as u32)
    }
    fn strict_eq(&self, a: &JsValue, b: &JsValue) -> bool {
        a == b
    }
    fn loose_eq(&mut self, a: &JsValue, b: &JsValue) -> Result<bool, Throw> {
        use JsValue::*;
        Ok(match (a, b) {
            (Undefined, Null) | (Null, Undefined) | (Undefined, Undefined) | (Null, Null) => true,
            (Num(x), Num(y)) => x == y,
            (Str(x), Str(y)) => x == y,
            (Bool(_), _) => {
                let n = a.to_number()?;
                self.loose_eq(&Num(n), b)?
            }
            (_, Bool(_)) => {
                let n = b.to_number()?;
                self.loose_eq(a, &Num(n))?
            }
            (Num(_) | Str(_), Object(_)) => {
                let p = self.primitive(b.clone())?;
                self.loose_eq(a, &p)?
            }
            (Object(_), Num(_) | Str(_)) => {
                let p = self.primitive(a.clone())?;
                self.loose_eq(&p, b)?
            }
            (Object(x), Object(y)) => Rc::ptr_eq(&x.0, &y.0),
            _ => false,
        })
    }
    fn instanceof(&mut self, a: &JsValue, b: &JsValue) -> Result<JsValue, Throw> {
        let r = match b {
            JsValue::Object(o) => o.clone(),
            _ => return Err(Throw::type_error("right operand is not callable")),
        };
        let p = match r.borrow().get_own("prototype") {
            Some(JsValue::Object(p)) => p,
            _ => return Err(Throw::type_error("prototype is not an object")),
        };
        if let (JsValue::Object(lhs), Some(fun)) = (a, r.borrow().func.clone()) {
            if let Some(JsValue::Str(lhs_name)) = lhs.borrow().get_own("name") {
                if !fun.name.is_empty() && lhs_name.as_ref() == fun.name {
                    return Ok(JsValue::Bool(true));
                }
            }
        }
        let mut c = match a {
            JsValue::Object(o) => o.borrow().proto.clone(),
            _ => None,
        };
        while let Some(o) = c {
            if Rc::ptr_eq(&o.0, &p.0) {
                return Ok(JsValue::Bool(true));
            }
            c = o.borrow().proto.clone()
        }
        Ok(JsValue::Bool(false))
    }

    fn native(name: &str, f: crate::value::NativeFn, proto: &Obj) -> JsValue {
        let mut o = JsObject::function(JsFunction {
            name: name.into(),
            params: Vec::new(),
            body: FunctionBody::Native(f),
            is_arrow: false,
            is_async: false,
            is_generator: false,
            env: EnvHandle::new(EnvKind::Global, None),
            home: None,
            proto_for_new: None,
        });
        o.proto = Some(proto.clone());
        let handle = Obj(Rc::new(RefCell::new(o)));
        let prototype = Obj(Rc::new(RefCell::new(JsObject::ordinary())));
        handle
            .borrow_mut()
            .set_own("prototype", JsValue::Object(prototype));
        handle.borrow_mut().set_own("name", JsValue::str(name));
        JsValue::Object(handle)
    }
    fn install_builtins(&mut self) {
        self.define_global("undefined", JsValue::Undefined);
        self.define_global("NaN", JsValue::Num(f64::NAN));
        self.define_global("Infinity", JsValue::Num(f64::INFINITY));
        self.define_global(
            "eval",
            Self::native("eval", native_eval, &self.function_proto),
        );
        self.define_global(
            "setTimeout",
            Self::native("setTimeout", native_set_timeout, &self.function_proto),
        );
        self.define_global(
            "clearTimeout",
            Self::native("clearTimeout", native_clear_timeout, &self.function_proto),
        );
        self.define_global(
            "Error",
            Self::native("Error", native_error, &self.function_proto),
        );
        self.define_global(
            "TypeError",
            Self::native("TypeError", native_type_error, &self.function_proto),
        );
        self.define_global(
            "RangeError",
            Self::native("RangeError", native_range_error, &self.function_proto),
        );
        self.define_global(
            "ReferenceError",
            Self::native(
                "ReferenceError",
                native_reference_error,
                &self.function_proto,
            ),
        );
        self.define_global(
            "SyntaxError",
            Self::native("SyntaxError", native_syntax_error, &self.function_proto),
        );
        let object_ctor = Self::native("Object", native_object, &self.function_proto);
        if let JsValue::Object(o) = &object_ctor {
            let _ = self.set_prop(
                &JsValue::Object(o.clone()),
                "keys",
                Self::native("keys", native_object_keys, &self.function_proto),
            );
            let _ = self.set_prop(
                &JsValue::Object(o.clone()),
                "defineProperty",
                Self::native(
                    "defineProperty",
                    native_define_property,
                    &self.function_proto,
                ),
            );
            let _ = self.set_prop(
                &JsValue::Object(o.clone()),
                "values",
                Self::native("values", native_object_values, &self.function_proto),
            );
            let _ = self.set_prop(
                &JsValue::Object(o.clone()),
                "assign",
                Self::native("assign", native_object_assign, &self.function_proto),
            );
            let _ = self.set_prop(
                &JsValue::Object(o.clone()),
                "preventExtensions",
                Self::native(
                    "preventExtensions",
                    native_prevent_extensions,
                    &self.function_proto,
                ),
            );
            let _ = self.set_prop(
                &JsValue::Object(o.clone()),
                "isExtensible",
                Self::native("isExtensible", native_is_extensible, &self.function_proto),
            );
            for (name, function) in [
                ("create", native_object_create as crate::value::NativeFn),
                (
                    "getPrototypeOf",
                    native_object_get_prototype_of as crate::value::NativeFn,
                ),
                (
                    "setPrototypeOf",
                    native_object_set_prototype_of as crate::value::NativeFn,
                ),
                ("hasOwn", native_object_has_own as crate::value::NativeFn),
            ] {
                let _ = self.set_prop(
                    &JsValue::Object(o.clone()),
                    name,
                    Self::native(name, function, &self.function_proto),
                );
            }
        }
        self.define_global("Object", object_ctor);
        self.define_global(
            "Array",
            Self::native("Array", native_array, &self.function_proto),
        );
        if let Some(array) = self.global_env.resolve("Array") {
            let _ = self.set_prop(
                &array,
                "isArray",
                Self::native("isArray", native_array_is_array, &self.function_proto),
            );
            let _ = self.set_prop(
                &array,
                "from",
                Self::native("from", native_array_from, &self.function_proto),
            );
            let _ = self.set_prop(
                &array,
                "of",
                Self::native("of", native_array_of, &self.function_proto),
            );
        }
        self.define_global("Map", Self::native("Map", native_map, &self.function_proto));
        self.define_global("Set", Self::native("Set", native_set, &self.function_proto));
        let symbol_ctor = Self::native("Symbol", native_symbol, &self.function_proto);
        if let JsValue::Object(symbol) = &symbol_ctor {
            let _ = self.set_prop(
                &JsValue::Object(symbol.clone()),
                "iterator",
                JsValue::str("@@iterator"),
            );
        }
        self.define_global("Symbol", symbol_ctor);
        let date_ctor = Self::native("Date", native_date, &self.function_proto);
        if let JsValue::Object(object) = &date_ctor {
            object
                .borrow_mut()
                .set_own("prototype", JsValue::Object(self.date_proto.clone()));
        }
        self.define_global("Date", date_ctor);
        let json = self.object_value();
        let _ = self.set_prop(
            &json,
            "parse",
            Self::native("parse", native_json_parse, &self.function_proto),
        );
        let _ = self.set_prop(
            &json,
            "stringify",
            Self::native("stringify", native_json_stringify, &self.function_proto),
        );
        self.define_global("JSON", json);
        self.define_global(
            "parseInt",
            Self::native("parseInt", native_parse_int, &self.function_proto),
        );
        self.define_global(
            "parseFloat",
            Self::native("parseFloat", native_parse_float, &self.function_proto),
        );
        self.define_global(
            "isFinite",
            Self::native("isFinite", native_is_finite, &self.function_proto),
        );
        let promise_ctor = Self::native("Promise", native_promise, &self.function_proto);
        if let JsValue::Object(o) = &promise_ctor {
            let _ = self.set_prop(
                &JsValue::Object(o.clone()),
                "resolve",
                Self::native("resolve", native_promise_resolve, &self.function_proto),
            );
            let _ = self.set_prop(
                &JsValue::Object(o.clone()),
                "reject",
                Self::native("reject", native_promise_reject, &self.function_proto),
            );
            let _ = self.set_prop(
                &JsValue::Object(o.clone()),
                "all",
                Self::native("all", native_promise_all, &self.function_proto),
            );
            let _ = self.set_prop(
                &JsValue::Object(o.clone()),
                "race",
                Self::native("race", native_promise_race, &self.function_proto),
            );
            let _ = self.set_prop(
                &JsValue::Object(o.clone()),
                "allSettled",
                Self::native(
                    "allSettled",
                    native_promise_all_settled,
                    &self.function_proto,
                ),
            );
            let _ = self.set_prop(
                &JsValue::Object(o.clone()),
                "any",
                Self::native("any", native_promise_any, &self.function_proto),
            );
        }
        self.define_global("Promise", promise_ctor);
        let _ = self.set_prop(
            &JsValue::Object(self.date_proto.clone()),
            "toString",
            Self::native("toString", native_date_to_string, &self.function_proto),
        );
        let _ = self.set_prop(
            &JsValue::Object(self.object_proto.clone()),
            "toString",
            Self::native("toString", native_object_to_string, &self.function_proto),
        );
        let _ = self.set_prop(
            &JsValue::Object(self.object_proto.clone()),
            "valueOf",
            Self::native("valueOf", native_object_value_of, &self.function_proto),
        );
        for (name, function) in [
            ("call", native_function_call as crate::value::NativeFn),
            ("apply", native_function_apply as crate::value::NativeFn),
            ("bind", native_function_bind as crate::value::NativeFn),
        ] {
            let _ = self.set_prop(
                &JsValue::Object(self.function_proto.clone()),
                name,
                Self::native(name, function, &self.function_proto),
            );
        }
        for (n, f) in [
            ("push", native_array_push as crate::value::NativeFn),
            ("pop", native_array_pop as crate::value::NativeFn),
            ("join", native_array_join as crate::value::NativeFn),
            ("map", native_array_map as crate::value::NativeFn),
            ("filter", native_array_filter as crate::value::NativeFn),
            ("forEach", native_array_for_each as crate::value::NativeFn),
            ("reduce", native_array_reduce as crate::value::NativeFn),
            ("slice", native_array_slice as crate::value::NativeFn),
            ("concat", native_array_concat as crate::value::NativeFn),
            ("includes", native_array_includes as crate::value::NativeFn),
            ("indexOf", native_array_index_of as crate::value::NativeFn),
        ] {
            let _ = self.set_prop(
                &JsValue::Object(self.array_proto.clone()),
                n,
                Self::native(n, f, &self.function_proto),
            );
        }
        for (n, f) in [
            ("then", native_promise_then as crate::value::NativeFn),
            ("catch", native_promise_catch as crate::value::NativeFn),
            ("finally", native_promise_finally as crate::value::NativeFn),
        ] {
            let _ = self.set_prop(
                &JsValue::Object(self.promise_proto.clone()),
                n,
                Self::native(n, f, &self.function_proto),
            );
        }
        for (n, f) in [
            ("get", native_map_get as crate::value::NativeFn),
            ("set", native_map_set as crate::value::NativeFn),
            ("has", native_map_has as crate::value::NativeFn),
            ("delete", native_map_delete as crate::value::NativeFn),
            ("clear", native_map_clear as crate::value::NativeFn),
            ("keys", native_map_keys as crate::value::NativeFn),
            ("values", native_map_values as crate::value::NativeFn),
            ("entries", native_map_entries as crate::value::NativeFn),
        ] {
            let _ = self.set_prop(
                &JsValue::Object(self.map_proto.clone()),
                n,
                Self::native(n, f, &self.function_proto),
            );
        }
        for (n, f) in [
            ("add", native_set_add as crate::value::NativeFn),
            ("has", native_set_has as crate::value::NativeFn),
            ("delete", native_set_delete as crate::value::NativeFn),
            ("clear", native_set_clear as crate::value::NativeFn),
            ("keys", native_set_values as crate::value::NativeFn),
            ("values", native_set_values as crate::value::NativeFn),
            ("entries", native_set_entries as crate::value::NativeFn),
        ] {
            let _ = self.set_prop(
                &JsValue::Object(self.set_proto.clone()),
                n,
                Self::native(n, f, &self.function_proto),
            );
        }
        let math = self.object_value();
        for (n, f) in [
            ("abs", native_abs as crate::value::NativeFn),
            ("max", native_max as crate::value::NativeFn),
            ("min", native_min as crate::value::NativeFn),
            ("floor", native_math_floor as crate::value::NativeFn),
            ("ceil", native_math_ceil as crate::value::NativeFn),
            ("round", native_math_round as crate::value::NativeFn),
            ("sqrt", native_math_sqrt as crate::value::NativeFn),
            ("pow", native_math_pow as crate::value::NativeFn),
            ("trunc", native_math_trunc as crate::value::NativeFn),
            ("sign", native_math_sign as crate::value::NativeFn),
        ] {
            let _ = self.set_prop(&math, n, Self::native(n, f, &self.function_proto));
        }
        self.define_global("Math", math);
        let console = self.object_value();
        let _ = self.set_prop(
            &console,
            "log",
            Self::native("log", native_log, &self.function_proto),
        );
        self.define_global("console", console);
        self.define_global(
            "Number",
            Self::native("Number", native_number, &self.function_proto),
        );
        self.define_global(
            "String",
            Self::native("String", native_string, &self.function_proto),
        );
        self.define_global(
            "Boolean",
            Self::native("Boolean", native_boolean, &self.function_proto),
        );
        self.define_global(
            "isNaN",
            Self::native("isNaN", native_is_nan, &self.function_proto),
        );
        if let Some(number) = self.global_env.resolve("Number") {
            let _ = self.set_prop(&number, "POSITIVE_INFINITY", JsValue::Num(f64::INFINITY));
            let _ = self.set_prop(
                &number,
                "NEGATIVE_INFINITY",
                JsValue::Num(f64::NEG_INFINITY),
            );
            let _ = self.set_prop(&number, "NaN", JsValue::Num(f64::NAN));
            let _ = self.set_prop(&number, "MAX_VALUE", JsValue::Num(f64::MAX));
            let _ = self.set_prop(&number, "MIN_VALUE", JsValue::Num(f64::from_bits(1)));
            let _ = self.set_prop(
                &number,
                "isNaN",
                Self::native("isNaN", native_number_is_nan, &self.function_proto),
            );
            let _ = self.set_prop(
                &number,
                "isFinite",
                Self::native("isFinite", native_number_is_finite, &self.function_proto),
            );
        }
    }
    fn define_global(&self, n: &str, v: JsValue) {
        self.global_obj.borrow_mut().set_own(n, v.clone());
        let _ = self.global_env.declare_var(n, v);
    }
}

fn reference_name(reference: &Reference) -> Option<String> {
    match reference {
        Reference::Binding(name) => Some(name.clone()),
        Reference::Property(_, key) => Some(key.clone()),
    }
}

fn binding_names(binding: &Binding) -> Vec<String> {
    match binding {
        Binding::Ident(identifier) => vec![identifier.name.clone()],
        Binding::Array(elements) => elements
            .iter()
            .flatten()
            .flat_map(|element| binding_names(element))
            .collect(),
        Binding::Object(properties) => properties
            .iter()
            .flat_map(|(_, property)| binding_names(property))
            .collect(),
        Binding::Rest(inner) | Binding::Default(inner, _) => binding_names(inner),
    }
}

fn is_destructuring_expr(expression: &ast::Expr) -> bool {
    match &expression.kind {
        ExprKind::Paren(inner) => is_destructuring_expr(inner),
        ExprKind::Array(_) | ExprKind::Object(_) => true,
        _ => false,
    }
}

fn for_iteration_env(left: &ForLeft, env: &EnvHandle) -> EnvHandle {
    match left {
        ForLeft::Decl(declaration) if declaration.kind != ast::VarKind::Var => {
            EnvHandle::new(EnvKind::Block, Some(env.clone()))
        }
        _ => env.clone(),
    }
}

fn native_object(i: &mut Interpreter, _: JsValue, a: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(a.into_iter()
        .next()
        .filter(|v| v.is_object())
        .unwrap_or_else(|| i.object_value()))
}

fn native_symbol(_: &mut Interpreter, _: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(JsValue::str("@@symbol"))
}

fn dom_id(this: &JsValue) -> Result<u32, Throw> {
    let JsValue::Object(object) = this else {
        return Err(Throw::type_error("DOM method called on non-element"));
    };
    match object.borrow().get_own("[[DomNode]]") {
        Some(JsValue::Num(n)) if n.is_finite() && n >= 0.0 && n.fract() == 0.0 => Ok(n as u32),
        _ => Err(Throw::type_error("DOM method called on non-element")),
    }
}

fn dom_host(i: &mut Interpreter) -> Result<Rc<RefCell<dyn DomHost>>, Throw> {
    i.dom_host
        .as_ref()
        .cloned()
        .ok_or_else(|| Throw::type_error("DOM host is not attached"))
}

fn native_dom_query_selector(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let selector = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    let id = dom_host(i)?.borrow_mut().query_selector(&selector);
    Ok(id.map(|id| i.make_dom_element(id)).unwrap_or(JsValue::Null))
}

fn native_dom_create_element(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let name = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    let id = dom_host(i)?
        .borrow_mut()
        .create_element(&name)
        .map_err(Throw::type_error)?;
    Ok(i.make_dom_element(id))
}

fn native_dom_text_get(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let id = dom_id(&this)?;
    Ok(dom_host(i)?
        .borrow()
        .get_text_content(id)
        .map(JsValue::str)
        .unwrap_or(JsValue::Undefined))
}

fn native_dom_text_set(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let id = dom_id(&this)?;
    let text = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    dom_host(i)?.borrow_mut().set_text_content(id, &text).map_err(Throw::type_error)?;
    Ok(JsValue::Undefined)
}

fn native_dom_get_attribute(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let id = dom_id(&this)?;
    let name = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    match dom_host(i)?.borrow().get_attribute(id, &name) {
        Some(Some(value)) => Ok(JsValue::str(value)),
        Some(None) => Ok(JsValue::Null),
        None => Ok(JsValue::Undefined),
    }
}

fn native_dom_set_attribute(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let id = dom_id(&this)?;
    let name = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    let value = i.to_string(args.get(1).unwrap_or(&JsValue::Undefined))?;
    dom_host(i)?.borrow_mut().set_attribute(id, &name, &value).map_err(Throw::type_error)?;
    Ok(JsValue::Undefined)
}

fn native_dom_append(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let parent = dom_id(&this)?;
    let host = dom_host(i)?;
    for arg in args {
        match arg {
            JsValue::Object(_) => {
                let child = dom_id(&arg)?;
                host.borrow_mut().append_child(parent, child).map_err(Throw::type_error)?;
            }
            other => {
                let text = i.to_string(&other)?;
                host.borrow_mut().append_text(parent, &text).map_err(Throw::type_error)?;
            }
        }
    }
    Ok(JsValue::Undefined)
}

fn native_dom_add_event_listener(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let node = dom_id(&this)?;
    let event = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    let callback = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    if callable_function(&callback).is_none() {
        return Err(Throw::type_error("event listener must be callable"));
    }
    i.register_dom_listener(node, event, callback);
    Ok(JsValue::Undefined)
}

fn native_dom_remove_event_listener(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let node = dom_id(&this)?;
    let event = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    let callback = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    i.dom_listeners.retain(|listener| {
        listener.property
            || listener.node != node
            || listener.event != event
            || listener.callback != callback
    });
    Ok(JsValue::Undefined)
}

fn native_dom_click(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let node = dom_id(&this)?;
    i.dispatch_dom_event(node, "click")?;
    Ok(JsValue::Undefined)
}

fn native_dom_onclick_get(
    i: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let node = dom_id(&this)?;
    Ok(i.dom_handler(node, "click").unwrap_or(JsValue::Null))
}

fn native_dom_onclick_set(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let node = dom_id(&this)?;
    let callback = args.first().cloned().unwrap_or(JsValue::Null);
    if !callback.is_null() && !callback.is_undefined() && callable_function(&callback).is_none() {
        return Err(Throw::type_error("onclick must be callable or null"));
    }
    let callback = if callback.is_null() || callback.is_undefined() {
        None
    } else {
        Some(callback)
    };
    i.set_dom_handler(node, "click", callback);
    Ok(JsValue::Undefined)
}

fn native_object_create(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let prototype = args.first().cloned().unwrap_or(JsValue::Null);
    let object = match prototype {
        JsValue::Null => JsObject::ordinary(),
        JsValue::Object(prototype) => {
            let mut object = JsObject::ordinary();
            object.proto = Some(prototype);
            object
        }
        _ => {
            return Err(Throw::type_error(
                "Object prototype must be an object or null",
            ))
        }
    };
    let result = JsValue::object(object);
    if let Some(properties) = args.get(1) {
        if !properties.is_null() && !properties.is_undefined() {
            let _ = native_object_assign(
                i,
                JsValue::Undefined,
                vec![result.clone(), properties.clone()],
            )?;
        }
    }
    Ok(result)
}

fn native_object_get_prototype_of(
    _: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    match args.first() {
        Some(JsValue::Object(object)) => Ok(object
            .borrow()
            .proto
            .clone()
            .map(JsValue::Object)
            .unwrap_or(JsValue::Null)),
        _ => Err(Throw::type_error("getPrototypeOf target is not an object")),
    }
}

fn native_object_set_prototype_of(
    _: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let object = match args.first() {
        Some(JsValue::Object(object)) => object.clone(),
        _ => return Err(Throw::type_error("setPrototypeOf target is not an object")),
    };
    let prototype = match args.get(1).cloned().unwrap_or(JsValue::Null) {
        JsValue::Null => None,
        JsValue::Object(prototype) => Some(prototype),
        _ => return Err(Throw::type_error("prototype must be an object or null")),
    };
    object.borrow_mut().proto = prototype;
    Ok(JsValue::Object(object))
}

fn native_object_has_own(
    _: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let object = match args.first() {
        Some(JsValue::Object(object)) => object,
        _ => return Err(Throw::type_error("hasOwn target is not an object")),
    };
    let key = match args.get(1) {
        Some(JsValue::Str(key)) => key.to_string(),
        Some(JsValue::Num(key)) => crate::value::number_to_string(*key),
        Some(JsValue::Bool(key)) => key.to_string(),
        Some(JsValue::Null) => "null".into(),
        _ => "undefined".into(),
    };
    Ok(JsValue::Bool(object.borrow().own_prop(&key).is_some()))
}

fn native_array_is_array(
    _: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    Ok(JsValue::Bool(
        matches!(args.first(), Some(JsValue::Object(object)) if matches!(object.borrow().exotic, Exotic::Array)),
    ))
}

fn native_array_from(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let source = args.first().cloned().unwrap_or(JsValue::Undefined);
    let mut values = i.iterate(&source)?;
    if let Some(callback) = args.get(1) {
        if callable_function(callback).is_none() {
            return Err(Throw::type_error("Array.from mapFn is not callable"));
        }
        for (index, value) in values.iter_mut().enumerate() {
            *value = i.call_value(
                callback,
                JsValue::Undefined,
                vec![value.clone(), JsValue::num(index as f64)],
            )?;
        }
    }
    array_values(i, values)
}

fn native_array_of(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    array_values(i, args)
}

fn native_number_is_nan(
    _: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    Ok(JsValue::Bool(
        matches!(args.first(), Some(JsValue::Num(value)) if value.is_nan()),
    ))
}

fn native_number_is_finite(
    _: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    Ok(JsValue::Bool(
        matches!(args.first(), Some(JsValue::Num(value)) if value.is_finite()),
    ))
}

fn native_function_call(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    if callable_function(&this).is_none() {
        return Err(Throw::type_error(
            "Function.prototype.call receiver is not callable",
        ));
    }
    let receiver = args.first().cloned().unwrap_or(JsValue::Undefined);
    i.call_value(&this, receiver, args.into_iter().skip(1).collect())
}

fn native_function_apply(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    if callable_function(&this).is_none() {
        return Err(Throw::type_error(
            "Function.prototype.apply receiver is not callable",
        ));
    }
    let receiver = args.first().cloned().unwrap_or(JsValue::Undefined);
    let arguments = match args.get(1) {
        None | Some(JsValue::Undefined) | Some(JsValue::Null) => Vec::new(),
        Some(value) => i.iterate(value)?,
    };
    i.call_value(&this, receiver, arguments)
}

fn native_function_bind(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    if callable_function(&this).is_none() {
        return Err(Throw::type_error(
            "Function.prototype.bind receiver is not callable",
        ));
    }
    let data = Obj(Rc::new(RefCell::new(JsObject::ordinary())));
    data.borrow_mut().set_own("target", this);
    data.borrow_mut()
        .set_own("this", args.first().cloned().unwrap_or(JsValue::Undefined));
    data.borrow_mut()
        .set_own("args", array_values(i, args.into_iter().skip(1).collect())?);
    let bound = Interpreter::native("bound", native_bound, &i.function_proto);
    if let JsValue::Object(function) = &bound {
        function.borrow_mut().func.as_mut().unwrap().home = Some(data);
    }
    Ok(bound)
}

fn native_bound(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = native_data(i)?;
    let target = data
        .borrow()
        .get_own("target")
        .ok_or_else(|| Throw::type_error("invalid bound function"))?;
    let receiver = data.borrow().get_own("this").unwrap_or(JsValue::Undefined);
    let bound_args = data
        .borrow()
        .get_own("args")
        .map(|value| i.iterate(&value))
        .transpose()?
        .unwrap_or_default();
    i.call_value(
        &target,
        receiver,
        bound_args.into_iter().chain(args).collect(),
    )
}

fn native_define_property(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let target = match args.first() {
        Some(JsValue::Object(object)) => object.clone(),
        _ => return Err(Throw::type_error("defineProperty target is not an object")),
    };
    let descriptor = match args.get(2) {
        Some(JsValue::Object(object)) => object.clone(),
        _ => return Err(Throw::type_error("property descriptor is not an object")),
    };
    let key = i.to_property_key(args.get(1).unwrap_or(&JsValue::Undefined))?;
    let has = |name: &str| descriptor.borrow().has_property(name);
    let accessor = has("get") || has("set");
    let data = has("value") || has("writable");
    if accessor && data {
        return Err(Throw::type_error(
            "descriptor mixes data and accessor fields",
        ));
    }
    if target.borrow().own_prop(&key).is_none() && !target.borrow().extensible {
        return Err(Throw::type_error("object is not extensible"));
    }
    let value = if has("value") {
        i.get_prop(&JsValue::Object(descriptor.clone()), "value")?
    } else {
        JsValue::Undefined
    };
    let writable = if has("writable") {
        i.get_prop(&JsValue::Object(descriptor.clone()), "writable")?
            .to_boolean()
    } else {
        false
    };
    let enumerable = if has("enumerable") {
        i.get_prop(&JsValue::Object(descriptor.clone()), "enumerable")?
            .to_boolean()
    } else {
        false
    };
    let configurable = if has("configurable") {
        i.get_prop(&JsValue::Object(descriptor.clone()), "configurable")?
            .to_boolean()
    } else {
        false
    };
    let getter = if has("get") {
        match i.get_prop(&JsValue::Object(descriptor.clone()), "get")? {
            JsValue::Undefined => None,
            value => Some(
                callable_function(&value)
                    .map(Box::new)
                    .ok_or_else(|| Throw::type_error("property getter is not callable"))?,
            ),
        }
    } else {
        None
    };
    let setter = if has("set") {
        match i.get_prop(&JsValue::Object(descriptor.clone()), "set")? {
            JsValue::Undefined => None,
            value => Some(
                callable_function(&value)
                    .map(Box::new)
                    .ok_or_else(|| Throw::type_error("property setter is not callable"))?,
            ),
        }
    } else {
        None
    };
    target
        .borrow_mut()
        .define(
            &key,
            Property {
                value,
                writable: if accessor { false } else { writable },
                enumerable,
                configurable,
                getter,
                setter,
            },
        )
        .map_err(|_| Throw::type_error("could not define property"))?;
    Ok(JsValue::Object(target))
}

fn native_prevent_extensions(
    _: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let target = args.first().cloned().unwrap_or(this);
    match target {
        JsValue::Object(object) => {
            object.borrow_mut().extensible = false;
            Ok(JsValue::Object(object))
        }
        _ => Err(Throw::type_error(
            "preventExtensions target is not an object",
        )),
    }
}

fn native_is_extensible(
    _: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    match args.first() {
        Some(JsValue::Object(object)) => Ok(JsValue::Bool(object.borrow().extensible)),
        _ => Err(Throw::type_error("isExtensible target is not an object")),
    }
}

fn native_date(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    if let JsValue::Object(object) = &this {
        let value = i.to_number(args.first().unwrap_or(&JsValue::num(0.0)))?;
        object
            .borrow_mut()
            .set_own("[[DateValue]]", JsValue::Num(value));
        Ok(this)
    } else {
        Ok(JsValue::str(""))
    }
}

fn native_date_to_string(
    _: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    if let JsValue::Object(object) = this {
        if object.borrow().get_own("[[DateValue]]").is_some() {
            return Ok(JsValue::str(
                "Thu Jan 01 1970 00:00:00 GMT+0000 (Coordinated Universal Time)",
            ));
        }
    }
    Ok(JsValue::str("Invalid Date"))
}

fn native_eval(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let source = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    let program =
        crate::parse_script(&source).map_err(|error| Throw::syntax_error(error.to_string()))?;
    i.run_program(&program)
}

fn native_error_value(i: &mut Interpreter, name: &str, args: Vec<JsValue>) -> JsValue {
    let message = args
        .first()
        .map(|value| i.to_string(value).unwrap_or_default())
        .unwrap_or_default();
    let object = Obj(Rc::new(RefCell::new({
        let mut object = JsObject::error_obj(name, JsValue::str(message));
        if let Some(function) = i.native_stack.last() {
            if let Some(JsValue::Object(proto)) = function.borrow().get_own("prototype") {
                object.proto = Some(proto);
            }
        }
        object
    })));
    JsValue::Object(object)
}

fn native_error(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(native_error_value(i, "Error", args))
}

fn native_type_error(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    Ok(native_error_value(i, "TypeError", args))
}

fn native_range_error(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    Ok(native_error_value(i, "RangeError", args))
}

fn native_reference_error(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    Ok(native_error_value(i, "ReferenceError", args))
}

fn native_syntax_error(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    Ok(native_error_value(i, "SyntaxError", args))
}

fn new_promise(i: &Interpreter, state: PromiseFate) -> JsValue {
    let promise = Obj(Rc::new(RefCell::new({
        let mut o = JsObject::ordinary();
        o.exotic = Exotic::Promise(PromiseState {
            state,
            ..PromiseState::default()
        });
        o.proto = Some(i.promise_proto.clone());
        o
    })));
    JsValue::Object(promise)
}

fn promise_object(value: &JsValue) -> Result<Obj, Throw> {
    match value {
        JsValue::Object(o) if matches!(o.borrow().exotic, Exotic::Promise(_)) => Ok(o.clone()),
        _ => Err(Throw::type_error(
            "Promise method called on a non-Promise object",
        )),
    }
}

fn callable_function(value: &JsValue) -> Option<JsFunction> {
    match value {
        JsValue::Object(o) => o.borrow().func.clone(),
        _ => None,
    }
}

fn promise_callback(
    i: &Interpreter,
    name: &str,
    target: &Obj,
    f: crate::value::NativeFn,
) -> JsValue {
    let value = Interpreter::native(name, f, &i.function_proto);
    if let JsValue::Object(o) = &value {
        o.borrow_mut()
            .set_own("[[PromiseTarget]]", JsValue::Object(target.clone()));
    }
    value
}

fn native_promise(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let executor = args.first().cloned().unwrap_or(JsValue::Undefined);
    if callable_function(&executor).is_none() {
        return Err(Throw::type_error("Promise resolver is not a function"));
    }
    let promise = promise_object(&new_promise(i, PromiseFate::Pending))?;
    let resolve = promise_callback(i, "resolve", &promise, native_promise_resolve);
    let reject = promise_callback(i, "reject", &promise, native_promise_reject);
    if let Err(Throw(value)) = i.call_value(&executor, JsValue::Undefined, vec![resolve, reject]) {
        settle_promise(&promise, PromiseFate::Rejected(value), i);
    }
    Ok(JsValue::Object(promise))
}

fn current_promise_target(i: &Interpreter) -> Option<Obj> {
    let function = i.native_stack.last()?;
    match function.borrow().get_own("[[PromiseTarget]]") {
        Some(JsValue::Object(target)) => Some(target),
        _ => None,
    }
}

fn native_promise_resolve(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    if let Some(target) = current_promise_target(i) {
        settle_promise(
            &target,
            PromiseFate::Fulfilled(args.first().cloned().unwrap_or(JsValue::Undefined)),
            i,
        );
        return Ok(JsValue::Undefined);
    }
    let value = args.first().cloned().unwrap_or(JsValue::Undefined);
    if matches!(value, JsValue::Object(ref o) if matches!(o.borrow().exotic, Exotic::Promise(_))) {
        Ok(value)
    } else {
        Ok(new_promise(i, PromiseFate::Fulfilled(value)))
    }
}

fn native_promise_reject(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    if let Some(target) = current_promise_target(i) {
        settle_promise(
            &target,
            PromiseFate::Rejected(args.first().cloned().unwrap_or(JsValue::Undefined)),
            i,
        );
        return Ok(JsValue::Undefined);
    }
    Ok(new_promise(
        i,
        PromiseFate::Rejected(args.first().cloned().unwrap_or(JsValue::Undefined)),
    ))
}

fn settle_promise(target: &Obj, fate: PromiseFate, i: &mut Interpreter) {
    let reactions = {
        let mut object = target.borrow_mut();
        let state = match &mut object.exotic {
            Exotic::Promise(state) if matches!(state.state, PromiseFate::Pending) => state,
            _ => return,
        };
        let fulfilled = matches!(fate, PromiseFate::Fulfilled(_));
        state.state = fate.clone();
        if fulfilled {
            std::mem::take(&mut state.fulfill_reactions)
        } else {
            std::mem::take(&mut state.reject_reactions)
        }
    };
    for reaction in reactions {
        i.jobs.push_back(PromiseJob {
            reaction,
            fate: fate.clone(),
        });
    }
}

fn run_promise_reaction(reaction: PendingReaction, fate: PromiseFate, i: &mut Interpreter) {
    let result = match (reaction.handler, fate) {
        (Some(handler), PromiseFate::Fulfilled(value)) => {
            let function = JsValue::Object(Obj(Rc::new(RefCell::new(JsObject::function(handler)))));
            match i.call_value(&function, JsValue::Undefined, vec![value]) {
                Ok(value) => PromiseFate::Fulfilled(value),
                Err(Throw(value)) => PromiseFate::Rejected(value),
            }
        }
        (Some(handler), PromiseFate::Rejected(value)) if reaction.is_reject_handler => {
            let function = JsValue::Object(Obj(Rc::new(RefCell::new(JsObject::function(handler)))));
            match i.call_value(&function, JsValue::Undefined, vec![value]) {
                Ok(value) => PromiseFate::Fulfilled(value),
                Err(Throw(value)) => PromiseFate::Rejected(value),
            }
        }
        (Some(_), PromiseFate::Rejected(value)) => PromiseFate::Rejected(value),
        (None, PromiseFate::Fulfilled(value)) => PromiseFate::Fulfilled(value),
        (None, PromiseFate::Rejected(value)) => PromiseFate::Rejected(value),
        (_, PromiseFate::Pending) => PromiseFate::Pending,
    };
    settle_promise(&reaction.capability, result, i);
}

fn native_promise_then(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let promise = promise_object(&this)?;
    let result = promise_object(&new_promise(i, PromiseFate::Pending))?;
    let fulfilled = args.get(0).and_then(callable_function);
    let rejected = args.get(1).and_then(callable_function);
    let state = {
        let mut object = promise.borrow_mut();
        let state = match &mut object.exotic {
            Exotic::Promise(state) => state,
            _ => unreachable!(),
        };
        match &state.state {
            PromiseFate::Pending => {
                state.fulfill_reactions.push(PendingReaction {
                    handler: fulfilled.clone(),
                    capability: result.clone(),
                    is_reject_handler: false,
                });
                state.reject_reactions.push(PendingReaction {
                    handler: rejected.clone(),
                    capability: result.clone(),
                    is_reject_handler: true,
                });
                None
            }
            PromiseFate::Fulfilled(value) => Some(PromiseFate::Fulfilled(value.clone())),
            PromiseFate::Rejected(value) => Some(PromiseFate::Rejected(value.clone())),
        }
    };
    if let Some(fate) = state {
        let reaction = if matches!(fate, PromiseFate::Fulfilled(_)) {
            PendingReaction {
                handler: fulfilled.clone(),
                capability: result.clone(),
                is_reject_handler: false,
            }
        } else {
            PendingReaction {
                handler: rejected.clone(),
                capability: result.clone(),
                is_reject_handler: true,
            }
        };
        i.jobs.push_back(PromiseJob { reaction, fate });
    }
    Ok(JsValue::Object(result))
}

fn native_promise_catch(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    native_promise_then(
        i,
        this,
        vec![
            JsValue::Undefined,
            args.first().cloned().unwrap_or(JsValue::Undefined),
        ],
    )
}

fn native_promise_finally(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let promise = promise_object(&this)?;
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    let state = match &promise.borrow().exotic {
        Exotic::Promise(state) => state.state.clone(),
        _ => unreachable!(),
    };
    if matches!(state, PromiseFate::Pending) {
        return Ok(this);
    }
    if callable_function(&callback).is_some() {
        i.call_value(&callback, JsValue::Undefined, Vec::new())?;
    }
    Ok(match state {
        PromiseFate::Fulfilled(value) => new_promise(i, PromiseFate::Fulfilled(value)),
        PromiseFate::Rejected(value) => new_promise(i, PromiseFate::Rejected(value)),
        PromiseFate::Pending => unreachable!(),
    })
}

fn native_data(i: &Interpreter) -> Result<Obj, Throw> {
    i.native_stack
        .last()
        .and_then(|function| function.borrow().func.as_ref()?.home.clone())
        .ok_or_else(|| Throw::type_error("missing native Promise callback state"))
}

fn combinator_callback(
    i: &Interpreter,
    name: &str,
    callback: crate::value::NativeFn,
    state: &Obj,
    index: usize,
) -> JsValue {
    let data = Obj(Rc::new(RefCell::new(JsObject::ordinary())));
    data.borrow_mut()
        .set_own("state", JsValue::Object(state.clone()));
    data.borrow_mut()
        .set_own("index", JsValue::num(index as f64));
    let value = Interpreter::native(name, callback, &i.function_proto);
    if let JsValue::Object(function) = &value {
        function.borrow_mut().func.as_mut().unwrap().home = Some(data);
    }
    value
}

fn combinator_state(i: &Interpreter) -> Result<(Obj, Obj, usize), Throw> {
    let data = native_data(i)?;
    let state = match data.borrow().get_own("state") {
        Some(JsValue::Object(state)) => state,
        _ => return Err(Throw::type_error("invalid Promise combinator state")),
    };
    let index = match data.borrow().get_own("index") {
        Some(JsValue::Num(index)) if index >= 0.0 => index as usize,
        _ => return Err(Throw::type_error("invalid Promise combinator index")),
    };
    let target = match state.borrow().get_own("target") {
        Some(JsValue::Object(target)) => target,
        _ => return Err(Throw::type_error("invalid Promise combinator target")),
    };
    Ok((state, target, index))
}

fn state_values(state: &Obj, name: &str) -> Result<JsValue, Throw> {
    state
        .borrow()
        .get_own(name)
        .ok_or_else(|| Throw::type_error("invalid Promise combinator values"))
}

fn state_remaining(state: &Obj) -> Result<usize, Throw> {
    match state.borrow().get_own("remaining") {
        Some(JsValue::Num(value)) if value >= 0.0 => Ok(value as usize),
        _ => Err(Throw::type_error("invalid Promise combinator count")),
    }
}

fn decrement_remaining(state: &Obj) -> Result<usize, Throw> {
    let remaining = state_remaining(state)?;
    let next = remaining.saturating_sub(1);
    state
        .borrow_mut()
        .set_own("remaining", JsValue::num(next as f64));
    Ok(next)
}

fn native_all_fulfill(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let (state, target, index) = combinator_state(i)?;
    let values = state_values(&state, "values")?;
    i.set_prop(
        &values,
        &index.to_string(),
        args.first().cloned().unwrap_or(JsValue::Undefined),
    )?;
    if decrement_remaining(&state)? == 0 {
        settle_promise(&target, PromiseFate::Fulfilled(values), i);
    }
    Ok(JsValue::Undefined)
}

fn native_all_reject(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let (_, target, _) = combinator_state(i)?;
    settle_promise(
        &target,
        PromiseFate::Rejected(args.first().cloned().unwrap_or(JsValue::Undefined)),
        i,
    );
    Ok(JsValue::Undefined)
}

fn native_race_fulfill(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let (_, target, _) = combinator_state(i)?;
    settle_promise(
        &target,
        PromiseFate::Fulfilled(args.first().cloned().unwrap_or(JsValue::Undefined)),
        i,
    );
    Ok(JsValue::Undefined)
}

fn native_race_reject(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let (_, target, _) = combinator_state(i)?;
    settle_promise(
        &target,
        PromiseFate::Rejected(args.first().cloned().unwrap_or(JsValue::Undefined)),
        i,
    );
    Ok(JsValue::Undefined)
}

fn settled_record(i: &Interpreter, status: &str, key: &str, value: JsValue) -> JsValue {
    let record = i.object_value();
    if let JsValue::Object(object) = &record {
        object.borrow_mut().set_own("status", JsValue::str(status));
        object.borrow_mut().set_own(key, value);
    }
    record
}

fn native_all_settled_fulfill(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let (state, target, index) = combinator_state(i)?;
    let values = state_values(&state, "values")?;
    i.set_prop(
        &values,
        &index.to_string(),
        settled_record(
            i,
            "fulfilled",
            "value",
            args.first().cloned().unwrap_or(JsValue::Undefined),
        ),
    )?;
    if decrement_remaining(&state)? == 0 {
        settle_promise(&target, PromiseFate::Fulfilled(values), i);
    }
    Ok(JsValue::Undefined)
}

fn native_all_settled_reject(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let (state, target, index) = combinator_state(i)?;
    let values = state_values(&state, "values")?;
    i.set_prop(
        &values,
        &index.to_string(),
        settled_record(
            i,
            "rejected",
            "reason",
            args.first().cloned().unwrap_or(JsValue::Undefined),
        ),
    )?;
    if decrement_remaining(&state)? == 0 {
        settle_promise(&target, PromiseFate::Fulfilled(values), i);
    }
    Ok(JsValue::Undefined)
}

fn native_any_fulfill(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let (_, target, _) = combinator_state(i)?;
    settle_promise(
        &target,
        PromiseFate::Fulfilled(args.first().cloned().unwrap_or(JsValue::Undefined)),
        i,
    );
    Ok(JsValue::Undefined)
}

fn native_any_reject(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let (state, target, index) = combinator_state(i)?;
    let errors = state_values(&state, "errors")?;
    i.set_prop(
        &errors,
        &index.to_string(),
        args.first().cloned().unwrap_or(JsValue::Undefined),
    )?;
    if decrement_remaining(&state)? == 0 {
        settle_promise(&target, PromiseFate::Rejected(errors), i);
    }
    Ok(JsValue::Undefined)
}

fn promise_input(i: &Interpreter, value: JsValue) -> JsValue {
    if matches!(value, JsValue::Object(ref object) if matches!(object.borrow().exotic, Exotic::Promise(_)))
    {
        value
    } else {
        new_promise(i, PromiseFate::Fulfilled(value))
    }
}

fn make_combinator_state(i: &mut Interpreter, count: usize, values: JsValue) -> (Obj, Obj) {
    let target = promise_object(&new_promise(i, PromiseFate::Pending)).unwrap();
    let state = Obj(Rc::new(RefCell::new(JsObject::ordinary())));
    state
        .borrow_mut()
        .set_own("target", JsValue::Object(target.clone()));
    state
        .borrow_mut()
        .set_own("remaining", JsValue::num(count as f64));
    state.borrow_mut().set_own("values", values);
    (state, target)
}

fn attach_combinator(
    i: &mut Interpreter,
    state: &Obj,
    input: JsValue,
    index: usize,
    fulfill: crate::value::NativeFn,
    reject: crate::value::NativeFn,
) -> Result<(), Throw> {
    let promise = promise_input(i, input);
    let on_fulfill = combinator_callback(i, "combinatorFulfill", fulfill, state, index);
    let on_reject = combinator_callback(i, "combinatorReject", reject, state, index);
    native_promise_then(i, promise, vec![on_fulfill, on_reject])?;
    Ok(())
}

fn native_promise_all(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let source = args
        .first()
        .ok_or_else(|| Throw::type_error("Promise.all requires an iterable"))?;
    let inputs = i.iterate(source)?;
    let values = array_values(i, vec![JsValue::Undefined; inputs.len()])?;
    let (state, target) = make_combinator_state(i, inputs.len(), values);
    if inputs.is_empty() {
        let values = state_values(&state, "values")?;
        settle_promise(&target, PromiseFate::Fulfilled(values), i);
    } else {
        for (index, input) in inputs.into_iter().enumerate() {
            attach_combinator(
                i,
                &state,
                input,
                index,
                native_all_fulfill,
                native_all_reject,
            )?;
        }
    }
    Ok(JsValue::Object(target))
}

fn native_promise_race(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let source = args
        .first()
        .ok_or_else(|| Throw::type_error("Promise.race requires an iterable"))?;
    let inputs = i.iterate(source)?;
    let values = array_values(i, Vec::new())?;
    let (state, target) = make_combinator_state(i, inputs.len(), values);
    for (index, input) in inputs.into_iter().enumerate() {
        attach_combinator(
            i,
            &state,
            input,
            index,
            native_race_fulfill,
            native_race_reject,
        )?;
    }
    Ok(JsValue::Object(target))
}

fn native_promise_all_settled(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let source = args
        .first()
        .ok_or_else(|| Throw::type_error("Promise.allSettled requires an iterable"))?;
    let inputs = i.iterate(source)?;
    let values = array_values(i, vec![JsValue::Undefined; inputs.len()])?;
    let (state, target) = make_combinator_state(i, inputs.len(), values);
    if inputs.is_empty() {
        let values = state_values(&state, "values")?;
        settle_promise(&target, PromiseFate::Fulfilled(values), i);
    } else {
        for (index, input) in inputs.into_iter().enumerate() {
            attach_combinator(
                i,
                &state,
                input,
                index,
                native_all_settled_fulfill,
                native_all_settled_reject,
            )?;
        }
    }
    Ok(JsValue::Object(target))
}

fn native_promise_any(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let source = args
        .first()
        .ok_or_else(|| Throw::type_error("Promise.any requires an iterable"))?;
    let inputs = i.iterate(source)?;
    let errors = array_values(i, vec![JsValue::Undefined; inputs.len()])?;
    let (state, target) = make_combinator_state(i, inputs.len(), errors.clone());
    state.borrow_mut().set_own("errors", errors);
    if inputs.is_empty() {
        settle_promise(
            &target,
            PromiseFate::Rejected(state_values(&state, "errors")?),
            i,
        );
    } else {
        for (index, input) in inputs.into_iter().enumerate() {
            attach_combinator(
                i,
                &state,
                input,
                index,
                native_any_fulfill,
                native_any_reject,
            )?;
        }
    }
    Ok(JsValue::Object(target))
}

fn map_key(value: &JsValue) -> MapKey {
    match value {
        JsValue::Undefined => MapKey::Undefined,
        JsValue::Null => MapKey::Null,
        JsValue::Bool(v) => MapKey::Bool(*v),
        JsValue::Num(v) => MapKey::Num(*v),
        JsValue::Str(v) => MapKey::Str(v.clone()),
        JsValue::Object(v) => MapKey::Object(v.clone()),
    }
}

fn require_map(value: &JsValue) -> Result<Obj, Throw> {
    match value {
        JsValue::Object(o) if matches!(o.borrow().exotic, Exotic::Map(_)) => Ok(o.clone()),
        _ => Err(Throw::type_error("Map method called on a non-Map object")),
    }
}

fn require_set(value: &JsValue) -> Result<Obj, Throw> {
    match value {
        JsValue::Object(o) if matches!(o.borrow().exotic, Exotic::Set(_)) => Ok(o.clone()),
        _ => Err(Throw::type_error("Set method called on a non-Set object")),
    }
}

fn array_values(i: &mut Interpreter, values: Vec<JsValue>) -> Result<JsValue, Throw> {
    let out = JsValue::object({
        let mut o = JsObject::array();
        o.proto = Some(i.array_proto.clone());
        o
    });
    for (n, value) in values.into_iter().enumerate() {
        i.set_prop(&out, &n.to_string(), value)?;
    }
    Ok(out)
}

fn map_set_raw(map: &Obj, key: MapKey, value: JsValue) {
    let mut object = map.borrow_mut();
    if let Exotic::Map(state) = &mut object.exotic {
        if let Some((_, slot)) = state.entries.iter_mut().find(|(k, _)| k.same(&key)) {
            *slot = value;
        } else {
            state.entries.push((key, value));
        }
    }
}

fn native_map(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let map = Obj(Rc::new(RefCell::new({
        let mut o = JsObject::ordinary();
        o.exotic = Exotic::Map(MapState::default());
        o.proto = Some(i.map_proto.clone());
        o
    })));
    if let Some(source) = args.first() {
        if !source.is_null() && !source.is_undefined() {
            for entry in i.iterate(source)? {
                let key = i.get_prop(&entry, "0")?;
                let value = i.get_prop(&entry, "1")?;
                map_set_raw(&map, map_key(&key), value);
            }
        }
    }
    Ok(JsValue::Object(map))
}

fn native_map_get(
    _: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let map = require_map(&this)?;
    let key = map_key(args.first().unwrap_or(&JsValue::Undefined));
    let object = map.borrow();
    if let Exotic::Map(state) = &object.exotic {
        if let Some((_, value)) = state.entries.iter().find(|(k, _)| k.same(&key)) {
            return Ok(value.clone());
        }
    }
    Ok(JsValue::Undefined)
}

fn native_map_set(
    _: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let map = require_map(&this)?;
    let key = map_key(args.first().unwrap_or(&JsValue::Undefined));
    let value = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    map_set_raw(&map, key, value);
    Ok(this)
}

fn native_map_has(
    _: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let map = require_map(&this)?;
    let key = map_key(args.first().unwrap_or(&JsValue::Undefined));
    let object = map.borrow();
    let found = matches!(&object.exotic, Exotic::Map(state) if state.entries.iter().any(|(k, _)| k.same(&key)));
    Ok(JsValue::Bool(found))
}

fn native_map_delete(
    _: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let map = require_map(&this)?;
    let key = map_key(args.first().unwrap_or(&JsValue::Undefined));
    let mut object = map.borrow_mut();
    let deleted = if let Exotic::Map(state) = &mut object.exotic {
        if let Some(pos) = state.entries.iter().position(|(k, _)| k.same(&key)) {
            state.entries.remove(pos);
            true
        } else {
            false
        }
    } else {
        false
    };
    Ok(JsValue::Bool(deleted))
}

fn native_map_clear(_: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let map = require_map(&this)?;
    if let Exotic::Map(state) = &mut map.borrow_mut().exotic {
        state.entries.clear();
    }
    Ok(JsValue::Undefined)
}

fn native_map_keys(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let map = require_map(&this)?;
    let values = if let Exotic::Map(state) = &map.borrow().exotic {
        state
            .entries
            .iter()
            .map(|(key, _)| key_value(key))
            .collect()
    } else {
        Vec::new()
    };
    array_values(i, values)
}

fn native_map_values(
    i: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let map = require_map(&this)?;
    let values = if let Exotic::Map(state) = &map.borrow().exotic {
        state
            .entries
            .iter()
            .map(|(_, value)| value.clone())
            .collect()
    } else {
        Vec::new()
    };
    array_values(i, values)
}

fn native_map_entries(
    i: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let map = require_map(&this)?;
    let entries = if let Exotic::Map(state) = &map.borrow().exotic {
        state
            .entries
            .iter()
            .map(|(key, value)| {
                array_values(i, vec![key_value(key), value.clone()]).unwrap_or(JsValue::Undefined)
            })
            .collect()
    } else {
        Vec::new()
    };
    array_values(i, entries)
}

fn key_value(key: &MapKey) -> JsValue {
    match key {
        MapKey::Undefined => JsValue::Undefined,
        MapKey::Null => JsValue::Null,
        MapKey::Bool(v) => JsValue::Bool(*v),
        MapKey::Num(v) => JsValue::Num(*v),
        MapKey::Str(v) => JsValue::Str(v.clone()),
        MapKey::Object(v) => JsValue::Object(v.clone()),
    }
}

fn native_set(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let set = Obj(Rc::new(RefCell::new({
        let mut o = JsObject::ordinary();
        o.exotic = Exotic::Set(SetState::default());
        o.proto = Some(i.set_proto.clone());
        o
    })));
    if let Some(source) = args.first() {
        if !source.is_null() && !source.is_undefined() {
            for value in i.iterate(source)? {
                let key = map_key(&value);
                if let Exotic::Set(state) = &mut set.borrow_mut().exotic {
                    if !state.entries.iter().any(|existing| existing.same(&key)) {
                        state.entries.push(key);
                    }
                }
            }
        }
    }
    Ok(JsValue::Object(set))
}

fn native_set_add(
    _: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let set = require_set(&this)?;
    let key = map_key(args.first().unwrap_or(&JsValue::Undefined));
    if let Exotic::Set(state) = &mut set.borrow_mut().exotic {
        if !state.entries.iter().any(|existing| existing.same(&key)) {
            state.entries.push(key);
        }
    }
    Ok(this)
}

fn native_set_has(
    _: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let set = require_set(&this)?;
    let key = map_key(args.first().unwrap_or(&JsValue::Undefined));
    let found = matches!(&set.borrow().exotic, Exotic::Set(state) if state.entries.iter().any(|existing| existing.same(&key)));
    Ok(JsValue::Bool(found))
}

fn native_set_delete(
    _: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let set = require_set(&this)?;
    let key = map_key(args.first().unwrap_or(&JsValue::Undefined));
    let mut object = set.borrow_mut();
    let deleted = if let Exotic::Set(state) = &mut object.exotic {
        if let Some(pos) = state
            .entries
            .iter()
            .position(|existing| existing.same(&key))
        {
            state.entries.remove(pos);
            true
        } else {
            false
        }
    } else {
        false
    };
    Ok(JsValue::Bool(deleted))
}

fn native_set_clear(_: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let set = require_set(&this)?;
    if let Exotic::Set(state) = &mut set.borrow_mut().exotic {
        state.entries.clear();
    }
    Ok(JsValue::Undefined)
}

fn native_set_values(
    i: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let set = require_set(&this)?;
    let values = if let Exotic::Set(state) = &set.borrow().exotic {
        state.entries.iter().map(key_value).collect()
    } else {
        Vec::new()
    };
    array_values(i, values)
}

fn native_set_entries(
    i: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let set = require_set(&this)?;
    let entries = if let Exotic::Set(state) = &set.borrow().exotic {
        state
            .entries
            .iter()
            .map(|key| {
                array_values(i, vec![key_value(key), key_value(key)]).unwrap_or(JsValue::Undefined)
            })
            .collect()
    } else {
        Vec::new()
    };
    array_values(i, entries)
}
fn native_object_keys(i: &mut Interpreter, _: JsValue, a: Vec<JsValue>) -> Result<JsValue, Throw> {
    let src = a.first().cloned().unwrap_or(JsValue::Undefined);
    let out = JsValue::object(JsObject::array());
    for (k, n) in i.enumerable_keys(&src).into_iter().enumerate() {
        i.set_prop(&out, &k.to_string(), JsValue::str(n))?;
    }
    Ok(out)
}

fn native_object_values(
    i: &mut Interpreter,
    _: JsValue,
    a: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let source = a.first().cloned().unwrap_or(JsValue::Undefined);
    let values = i
        .enumerable_keys(&source)
        .into_iter()
        .map(|key| i.get_prop(&source, &key))
        .collect::<Result<Vec<_>, _>>()?;
    array_values(i, values)
}

fn native_object_assign(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let target = match args.first() {
        Some(JsValue::Object(_)) => args[0].clone(),
        _ => return Err(Throw::type_error("Object.assign target is not an object")),
    };
    for source in args.iter().skip(1) {
        for key in i.enumerable_keys(source) {
            let value = i.get_prop(source, &key)?;
            i.set_prop(&target, &key, value)?;
        }
    }
    Ok(target)
}

fn native_json_parse(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let source = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    let program = crate::parse_script(&format!("({source})"))
        .map_err(|error| Throw::syntax_error(error.to_string()))?;
    i.run_program(&program)
}

fn json_escape(value: &str) -> String {
    let mut output = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            c if c.is_control() => output.push_str(&format!("\\u{:04x}", c as u32)),
            c => output.push(c),
        }
    }
    output.push('"');
    output
}

fn json_stringify_value(i: &mut Interpreter, value: &JsValue) -> Result<Option<String>, Throw> {
    Ok(match value {
        JsValue::Undefined => None,
        JsValue::Null => Some("null".into()),
        JsValue::Bool(value) => Some(value.to_string()),
        JsValue::Num(value) if value.is_finite() => Some(crate::value::number_to_string(*value)),
        JsValue::Num(_) => Some("null".into()),
        JsValue::Str(value) => Some(json_escape(value)),
        JsValue::Object(object) if object.borrow().func.is_some() => None,
        JsValue::Object(object) if matches!(object.borrow().exotic, Exotic::Array) => {
            let mut parts = Vec::new();
            for index in 0..i.array_length(value) {
                let element = i.get_prop(value, &index.to_string())?;
                parts.push(json_stringify_value(i, &element)?.unwrap_or_else(|| "null".into()));
            }
            Some(format!("[{}]", parts.join(",")))
        }
        JsValue::Object(object) => {
            let mut parts = Vec::new();
            for key in object.borrow().own_keys_enumerable() {
                let property = i.get_prop(value, &key)?;
                if let Some(value) = json_stringify_value(i, &property)? {
                    parts.push(format!("{}:{}", json_escape(&key), value));
                }
            }
            Some(format!("{{{}}}", parts.join(",")))
        }
    })
}

fn native_json_stringify(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let value = args.first().unwrap_or(&JsValue::Undefined);
    Ok(json_stringify_value(i, value)?
        .map(JsValue::str)
        .unwrap_or(JsValue::Undefined))
}

fn native_parse_int(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let mut source = i
        .to_string(args.first().unwrap_or(&JsValue::Undefined))?
        .trim()
        .to_string();
    let negative = source.starts_with('-');
    if negative || source.starts_with('+') {
        source.remove(0);
    }
    let mut radix = i.to_number(args.get(1).unwrap_or(&JsValue::num(0.0)))? as i32;
    if radix == 0 {
        radix = if source.starts_with("0x") || source.starts_with("0X") {
            16
        } else {
            10
        };
    }
    if !(2..=36).contains(&radix) {
        return Ok(JsValue::num(f64::NAN));
    }
    if radix == 16 && (source.starts_with("0x") || source.starts_with("0X")) {
        source.drain(..2);
    }
    let digits: String = source
        .chars()
        .take_while(|ch| ch.to_digit(radix as u32).is_some())
        .collect();
    if digits.is_empty() {
        return Ok(JsValue::num(f64::NAN));
    }
    let number = u128::from_str_radix(&digits, radix as u32).unwrap_or(0) as f64;
    Ok(JsValue::num(if negative { -number } else { number }))
}

fn native_parse_float(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let source = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    let trimmed = source.trim();
    if trimmed.starts_with("Infinity") {
        return Ok(JsValue::num(f64::INFINITY));
    }
    let mut end = 0usize;
    for (index, ch) in trimmed.char_indices() {
        if ch.is_ascii_digit() || matches!(ch, '+' | '-' | '.' | 'e' | 'E') {
            end = index + ch.len_utf8();
        } else {
            break;
        }
    }
    Ok(JsValue::num(
        trimmed[..end].parse::<f64>().unwrap_or(f64::NAN),
    ))
}

fn native_is_finite(_: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    match args.first().unwrap_or(&JsValue::Undefined) {
        JsValue::Num(value) => Ok(JsValue::Bool(value.is_finite())),
        _ => Ok(JsValue::Bool(false)),
    }
}
fn native_object_to_string(
    i: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let s = match this {
        JsValue::Object(o) if matches!(o.borrow().exotic, Exotic::Array) => "[object Array]",
        JsValue::Object(_) => "[object Object]",
        JsValue::Null => "[object Null]",
        JsValue::Undefined => "[object Undefined]",
        _ => "[object Object]",
    };
    let _ = i;
    Ok(JsValue::str(s))
}
fn native_object_value_of(
    _: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    Ok(this)
}
fn native_array_push(
    i: &mut Interpreter,
    this: JsValue,
    a: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let mut n = i.array_length(&this);
    for v in a {
        i.set_prop(&this, &n.to_string(), v)?;
        n += 1;
    }
    Ok(JsValue::num(n as f64))
}
fn native_array_pop(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let n = i.array_length(&this);
    if n == 0 {
        return Ok(JsValue::Undefined);
    }
    let k = (n - 1).to_string();
    let v = i.get_prop(&this, &k)?;
    i.set_prop(&this, "length", JsValue::num((n - 1) as f64))?;
    if let JsValue::Object(o) = this {
        o.borrow_mut().delete_own(&k);
    }
    Ok(v)
}
fn native_array_join(
    i: &mut Interpreter,
    this: JsValue,
    a: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let sep = if let Some(v) = a.first() {
        i.to_string(v)?
    } else {
        " ,".trim().to_string()
    };
    let mut out = String::new();
    for n in 0..i.array_length(&this) {
        if n > 0 {
            out.push_str(&sep)
        }
        let v = i.get_prop(&this, &n.to_string())?;
        if !v.is_null() && !v.is_undefined() {
            out.push_str(&i.to_string(&v)?);
        }
    }
    Ok(JsValue::str(out))
}
fn native_array(i: &mut Interpreter, _: JsValue, a: Vec<JsValue>) -> Result<JsValue, Throw> {
    let v = JsValue::object({
        let mut o = JsObject::array();
        o.proto = Some(i.array_proto.clone());
        o
    });
    for (n, x) in a.into_iter().enumerate() {
        i.set_prop(&v, &n.to_string(), x)?;
    }
    Ok(v)
}
fn native_abs(i: &mut Interpreter, _: JsValue, a: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(JsValue::num(
        i.to_number(a.first().unwrap_or(&JsValue::Undefined))?.abs(),
    ))
}
fn math_arg(i: &mut Interpreter, args: &[JsValue]) -> Result<f64, Throw> {
    i.to_number(args.first().unwrap_or(&JsValue::Undefined))
}
fn native_math_floor(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    Ok(JsValue::num(math_arg(i, &args)?.floor()))
}
fn native_math_ceil(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(JsValue::num(math_arg(i, &args)?.ceil()))
}
fn native_math_round(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let value = math_arg(i, &args)?;
    Ok(JsValue::num(if value.fract() == -0.5 {
        value.ceil()
    } else {
        value.round()
    }))
}
fn native_math_sqrt(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(JsValue::num(math_arg(i, &args)?.sqrt()))
}
fn native_math_pow(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let base = i.to_number(args.first().unwrap_or(&JsValue::Undefined))?;
    let exponent = i.to_number(args.get(1).unwrap_or(&JsValue::Undefined))?;
    Ok(JsValue::num(base.powf(exponent)))
}
fn native_math_trunc(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    Ok(JsValue::num(math_arg(i, &args)?.trunc()))
}
fn native_math_sign(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let value = math_arg(i, &args)?;
    Ok(JsValue::num(if value.is_nan() {
        f64::NAN
    } else if value == 0.0 {
        value
    } else if value < 0.0 {
        -1.0
    } else {
        1.0
    }))
}
fn native_max(i: &mut Interpreter, _: JsValue, a: Vec<JsValue>) -> Result<JsValue, Throw> {
    let mut n = f64::NEG_INFINITY;
    for x in &a {
        n = n.max(i.to_number(x)?);
    }
    Ok(JsValue::num(n))
}
fn native_min(i: &mut Interpreter, _: JsValue, a: Vec<JsValue>) -> Result<JsValue, Throw> {
    let mut n = f64::INFINITY;
    for x in &a {
        n = n.min(i.to_number(x)?);
    }
    Ok(JsValue::num(n))
}
fn native_number(i: &mut Interpreter, this: JsValue, a: Vec<JsValue>) -> Result<JsValue, Throw> {
    let value = JsValue::num(i.to_number(a.first().unwrap_or(&JsValue::Undefined))?);
    if let JsValue::Object(object) = &this {
        object.borrow_mut().set_own("[[NumberData]]", value);
        Ok(this)
    } else {
        Ok(value)
    }
}
fn native_string(i: &mut Interpreter, this: JsValue, a: Vec<JsValue>) -> Result<JsValue, Throw> {
    let value = JsValue::str(i.to_string(a.first().unwrap_or(&JsValue::Undefined))?);
    if let JsValue::Object(object) = &this {
        object.borrow_mut().set_own("[[StringData]]", value);
        Ok(this)
    } else {
        Ok(value)
    }
}
fn native_string_split(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let source = match this {
        JsValue::Str(value) => value.to_string(),
        value => i.to_string(&value)?,
    };
    let separator = args.first().cloned().unwrap_or(JsValue::Undefined);
    let parts: Vec<String> = if separator.is_undefined() {
        vec![source]
    } else {
        let separator = i.to_string(&separator)?;
        if separator.is_empty() {
            source.chars().map(|c| c.to_string()).collect()
        } else {
            source.split(&separator).map(str::to_string).collect()
        }
    };
    let values = parts.into_iter().map(JsValue::str).collect();
    array_values(i, values)
}

fn string_this(i: &mut Interpreter, this: JsValue) -> Result<String, Throw> {
    match this {
        JsValue::Str(value) => Ok(value.to_string()),
        value => i.to_string(&value),
    }
}

fn native_string_trim(
    i: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    Ok(JsValue::str(string_this(i, this)?.trim().to_string()))
}

fn native_string_char_at(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let source = string_this(i, this)?;
    let index = i.to_number(args.first().unwrap_or(&JsValue::num(0.0)))?;
    if !index.is_finite() || index < 0.0 {
        return Ok(JsValue::str(""));
    }
    Ok(source
        .chars()
        .nth(index.trunc() as usize)
        .map(|c| JsValue::str(c.to_string()))
        .unwrap_or_else(|| JsValue::str("")))
}

fn native_string_index_of(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let source = string_this(i, this)?;
    let needle = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    Ok(JsValue::num(
        source.find(&needle).map(|n| n as f64).unwrap_or(-1.0),
    ))
}

fn native_string_includes(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let source = string_this(i, this)?;
    let needle = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    Ok(JsValue::Bool(source.contains(&needle)))
}

fn native_string_replace(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let source = string_this(i, this)?;
    let search = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    let replacement = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    if let Some(position) = source.find(&search) {
        let replacement = if callable_function(&replacement).is_some() {
            let result = i.call_value(
                &replacement,
                JsValue::Undefined,
                vec![JsValue::str(search.clone())],
            )?;
            i.to_string(&result)?
        } else {
            i.to_string(&replacement)?
        };
        let mut output = String::new();
        output.push_str(&source[..position]);
        output.push_str(&replacement);
        output.push_str(&source[position + search.len()..]);
        Ok(JsValue::str(output))
    } else {
        Ok(JsValue::str(source))
    }
}

fn native_string_starts_with(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let source = string_this(i, this)?;
    let search = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    Ok(JsValue::Bool(source.starts_with(&search)))
}

fn native_string_ends_with(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let source = string_this(i, this)?;
    let search = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    Ok(JsValue::Bool(source.ends_with(&search)))
}

fn array_callback(
    i: &mut Interpreter,
    this: JsValue,
    args: &[JsValue],
    mut each: impl FnMut(&mut Interpreter, usize, JsValue, JsValue) -> Result<(), Throw>,
) -> Result<JsValue, Throw> {
    let callback = args
        .first()
        .ok_or_else(|| Throw::type_error("array callback is not a function"))?;
    if callable_function(callback).is_none() {
        return Err(Throw::type_error("array callback is not a function"));
    }
    let length = i.array_length(&this);
    for index in 0..length {
        let value = i.get_prop(&this, &index.to_string())?;
        let result = i.call_value(
            callback,
            args.get(1).cloned().unwrap_or(JsValue::Undefined),
            vec![value.clone(), JsValue::num(index as f64), this.clone()],
        )?;
        each(i, index, value, result)?;
    }
    Ok(JsValue::Undefined)
}

fn native_array_map(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let out = JsValue::object({
        let mut object = JsObject::array();
        object.proto = Some(i.array_proto.clone());
        object
    });
    array_callback(i, this, &args, |i, index, _, result| {
        i.set_prop(&out, &index.to_string(), result)
    })?;
    Ok(out)
}

fn native_array_filter(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let out = JsValue::object({
        let mut object = JsObject::array();
        object.proto = Some(i.array_proto.clone());
        object
    });
    let mut next = 0usize;
    array_callback(i, this, &args, |i, _, value, selected| {
        if selected.to_boolean() {
            i.set_prop(&out, &next.to_string(), value)?;
            next += 1;
        }
        Ok(())
    })?;
    Ok(out)
}

fn native_array_for_each(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    array_callback(i, this, &args, |_, _, _, _| Ok(()))
}

fn native_array_reduce(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let callback = args
        .first()
        .ok_or_else(|| Throw::type_error("array reducer is not a function"))?;
    if callable_function(callback).is_none() {
        return Err(Throw::type_error("array reducer is not a function"));
    }
    let length = i.array_length(&this);
    let mut index = 0usize;
    let mut accumulator = if args.len() > 1 {
        args[1].clone()
    } else if length > 0 {
        let value = i.get_prop(&this, "0")?;
        index = 1;
        value
    } else {
        return Err(Throw::type_error(
            "reduce of empty array with no initial value",
        ));
    };
    while index < length {
        let value = i.get_prop(&this, &index.to_string())?;
        accumulator = i.call_value(
            callback,
            JsValue::Undefined,
            vec![accumulator, value, JsValue::num(index as f64), this.clone()],
        )?;
        index += 1;
    }
    Ok(accumulator)
}

fn native_array_slice(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let length = i.array_length(&this) as i64;
    let start = i
        .to_number(args.first().unwrap_or(&JsValue::num(0.0)))?
        .trunc() as i64;
    let end = i
        .to_number(args.get(1).unwrap_or(&JsValue::num(length as f64)))?
        .trunc() as i64;
    let start = if start < 0 {
        (length + start).max(0)
    } else {
        start.min(length)
    } as usize;
    let end = if end < 0 {
        (length + end).max(0)
    } else {
        end.min(length)
    } as usize;
    let mut values = Vec::new();
    for index in start..end.max(start) {
        values.push(i.get_prop(&this, &index.to_string())?);
    }
    array_values(i, values)
}

fn native_array_concat(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let mut values = i.iterate(&this)?;
    for argument in args {
        if matches!(argument, JsValue::Object(ref object) if matches!(object.borrow().exotic, Exotic::Array))
            || matches!(argument, JsValue::Str(_))
        {
            values.extend(i.iterate(&argument)?);
        } else {
            values.push(argument);
        }
    }
    array_values(i, values)
}

fn native_array_includes(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let search = args.first().cloned().unwrap_or(JsValue::Undefined);
    Ok(JsValue::Bool(
        i.iterate(&this)?.into_iter().any(|value| value == search),
    ))
}

fn native_array_index_of(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let search = args.first().cloned().unwrap_or(JsValue::Undefined);
    for (index, value) in i.iterate(&this)?.into_iter().enumerate() {
        if value == search {
            return Ok(JsValue::num(index as f64));
        }
    }
    Ok(JsValue::num(-1.0))
}
fn native_boolean(_: &mut Interpreter, this: JsValue, a: Vec<JsValue>) -> Result<JsValue, Throw> {
    let value = JsValue::Bool(a.first().map(|x| x.to_boolean()).unwrap_or(false));
    if let JsValue::Object(object) = &this {
        object.borrow_mut().set_own("[[BooleanData]]", value);
        Ok(this)
    } else {
        Ok(value)
    }
}
fn native_is_nan(i: &mut Interpreter, _: JsValue, a: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(JsValue::Bool(
        i.to_number(a.first().unwrap_or(&JsValue::Undefined))?
            .is_nan(),
    ))
}
fn native_log(i: &mut Interpreter, _: JsValue, a: Vec<JsValue>) -> Result<JsValue, Throw> {
    let mut s = String::new();
    for (n, x) in a.iter().enumerate() {
        if n > 0 {
            s.push(' ')
        }
        s.push_str(&i.to_string(x)?);
    }
    i.output.borrow_mut().push(s);
    Ok(JsValue::Undefined)
}

fn native_set_timeout(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    if callable_function(&callback).is_none() {
        return Err(Throw::type_error("setTimeout callback is not callable"));
    }
    let delay = i
        .to_number(args.get(1).unwrap_or(&JsValue::num(0.0)))?
        .max(0.0)
        .min(u64::MAX as f64) as u64;
    let id = i.schedule_timer(callback, delay, args.into_iter().skip(2).collect());
    Ok(JsValue::num(id as f64))
}

fn native_clear_timeout(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let id = i
        .to_number(args.first().unwrap_or(&JsValue::Undefined))?
        .max(0.0) as u32;
    i.cancel_timer(id);
    Ok(JsValue::Undefined)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn num(v: JsValue) -> f64 {
        match v {
            JsValue::Num(n) => n,
            _ => panic!("expected number: {v:?}"),
        }
    }

    #[test]
    fn evaluates_arithmetic_and_coercion() {
        let mut i = Interpreter::new();
        assert_eq!(num(i.run_script("1 + 2 * 3").unwrap()), 7.0);
        let v = i.run_script("'a' + 2").unwrap();
        assert_eq!(i.to_string(&v).unwrap(), "a2");
    }

    #[test]
    fn closures_capture_and_mutate_lexical_bindings() {
        let mut i = Interpreter::new();
        let v = i
            .run_script("function make(){ let n=0; return ()=>++n; } let f=make(); f()+f();")
            .unwrap();
        assert_eq!(num(v), 3.0);
    }

    #[test]
    fn arrays_objects_and_builtin_methods_work() {
        let mut i = Interpreter::new();
        let v = i
            .run_script("let a=[1,2]; a.push(3); a.join('-');")
            .unwrap();
        assert_eq!(i.to_string(&v).unwrap(), "1-2-3");
        let v = i.run_script("Object.keys({a:1,b:2}).length").unwrap();
        assert_eq!(num(v), 2.0);
    }

    #[test]
    fn common_array_string_and_object_utilities_work() {
        let mut i = Interpreter::new();
        let value = i
            .run_script("[1,2,3].map(x=>x*2).filter(x=>x>2).reduce((a,b)=>a+b,0);")
            .unwrap();
        assert_eq!(num(value), 10.0);
        let value = i
            .run_script("'  hello  '.trim().includes('ell') && 'abc'.charAt(1)==='b' && 'abc'.indexOf('c')===2;")
            .unwrap();
        assert!(matches!(value, JsValue::Bool(true)));
        let value = i.run_script("Object.values({a:1,b:2}).join(',');").unwrap();
        assert_eq!(i.to_string(&value).unwrap(), "1,2");
        let value = i
            .run_script("[1,2,3,4].slice(1,3).concat([5]).includes(5) && [1,2,3].indexOf(2)===1 && 'abcdef'.replace('cd','X')==='abXef' && 'abc'.startsWith('a') && 'abc'.endsWith('c');")
            .unwrap();
        assert!(matches!(value, JsValue::Bool(true)));
        let mut k = Interpreter::new();
        let value = k
            .run_script("Array.from('ab').concat(Array.of(3)).join(',') === 'a,b,3' && Number.isNaN(NaN) && Number.isFinite(3);")
            .unwrap();
        assert!(matches!(value, JsValue::Bool(true)));
        let value = k
            .run_script("Math.floor(1.9)===1 && Math.ceil(1.1)===2 && Math.round(1.5)===2 && Math.sqrt(9)===3 && Math.pow(2,3)===8 && Math.trunc(-1.9)===-1 && Math.sign(-2)===-1;")
            .unwrap();
        assert!(matches!(value, JsValue::Bool(true)));
        let mut j = Interpreter::new();
        let value = j
            .run_script("JSON.stringify({a:1,b:[2]}) === '{\"a\":1,\"b\":[2]}' && JSON.parse('{\"x\":4}').x === 4 && parseInt('0x10') === 16 && parseFloat('1.5px') === 1.5;")
            .unwrap();
        assert!(matches!(value, JsValue::Bool(true)));
    }

    #[test]
    fn function_call_apply_and_bind_preserve_receiver_and_arguments() {
        let mut i = Interpreter::new();
        let value = i
            .run_script("function f(a,b){return this.x+a+b;} let o={x:1}; f.call(o,2,3)+f.apply(o,[2,3])+f.bind(o,2)(3);")
            .unwrap();
        assert_eq!(num(value), 18.0);
    }

    #[test]
    fn object_prototype_helpers_and_array_is_array_work() {
        let mut i = Interpreter::new();
        let value = i
            .run_script("let p={x:4}; let o=Object.create(p); o.y=2; Object.getPrototypeOf(o)===p && Object.hasOwn(o,'y') && !Object.hasOwn(o,'x') && Array.isArray([]) && !Array.isArray({});")
            .unwrap();
        assert!(matches!(value, JsValue::Bool(true)));
    }

    #[test]
    fn class_inheritance_dispatches_super_methods_with_the_real_receiver() {
        let mut i = Interpreter::new();
        let value = i
            .run_script("class A { value(){ return 4; } } class B extends A { value(){ return super.value()+1; } } new B().value();")
            .unwrap();
        assert_eq!(num(value), 5.0);
    }

    #[test]
    fn derived_constructor_can_call_super() {
        let mut i = Interpreter::new();
        let value = i
            .run_script("class A { constructor(value){ this.value=value; } } class B extends A { constructor(value){ super(value+1); } } new B(4).value;")
            .unwrap();
        assert_eq!(num(value), 5.0);
    }

    #[test]
    fn class_getters_and_setters_use_property_descriptors() {
        let mut i = Interpreter::new();
        let value = i
            .run_script("class C { constructor(){this._x=1;} get x(){return this._x;} set x(v){this._x=v;} } let c=new C(); c.x=7; c.x;")
            .unwrap();
        assert_eq!(num(value), 7.0);
    }

    #[test]
    fn map_uses_same_value_zero_and_preserves_insertion_order() {
        let mut i = Interpreter::new();
        let v = i
            .run_script(
                "let m=new Map([['a',1],['b',2]]); m.set('a',9).set('c',3); m.get('a') + m.size;",
            )
            .unwrap();
        assert_eq!(num(v), 12.0);
        let v = i
            .run_script("new Map([['a',1],['b',2]]).keys().join(',');")
            .unwrap();
        assert_eq!(i.to_string(&v).unwrap(), "a,b");
        let v = i
            .run_script("let m2=new Map(); m2.set(NaN,1); m2.has(NaN);")
            .unwrap();
        assert!(matches!(v, JsValue::Bool(true)));
    }

    #[test]
    fn set_deduplicates_same_value_zero_and_supports_mutation() {
        let mut i = Interpreter::new();
        let v = i
            .run_script("let s=new Set([1,1,2,NaN,NaN]); s.add(3); s.delete(2); s.size;")
            .unwrap();
        assert_eq!(num(v), 3.0);
        let v = i
            .run_script("new Set(['x','y']).values().join('-');")
            .unwrap();
        assert_eq!(i.to_string(&v).unwrap(), "x-y");
    }

    #[test]
    fn settled_promises_chain_and_catch_synchronously() {
        let mut i = Interpreter::new();
        i.run_script(
            "Promise.resolve(2).then(x=>x+1).then(x=>console.log(x)); Promise.reject('bad').catch(e=>console.log(e));",
        )
        .unwrap();
        assert_eq!(&*i.output.borrow(), &["bad".to_string(), "3".to_string()]);
    }

    #[test]
    fn pending_promise_reactions_run_when_resolved() {
        let mut i = Interpreter::new();
        i.run_script(
            "let resolve; let p=new Promise(r=>{resolve=r;}); p.then(x=>console.log(x)); resolve(5);",
        )
        .unwrap();
        assert_eq!(&*i.output.borrow(), &["5".to_string()]);
    }

    #[test]
    fn async_function_returns_a_promise() {
        let mut i = Interpreter::new();
        i.run_script("async function f(){ return 4; } f().then(x=>console.log(x+1));")
            .unwrap();
        assert_eq!(&*i.output.borrow(), &["5".to_string()]);
    }

    #[test]
    fn promise_reactions_run_after_current_script_turn() {
        let mut i = Interpreter::new();
        i.run_script(
            "console.log('before'); Promise.resolve(1).then(x=>console.log(x)); console.log('after');",
        )
        .unwrap();
        assert_eq!(
            &*i.output.borrow(),
            &["before".to_string(), "after".to_string(), "1".to_string()]
        );
    }

    #[test]
    fn eval_wrappers_and_number_constants_follow_primitive_rules() {
        let mut i = Interpreter::new();
        let value = i
            .run_script("eval('1 + 1') + new Boolean(true) + Number.MIN_VALUE > 2;")
            .unwrap();
        assert!(matches!(value, JsValue::Bool(true)));
        let value = i
            .run_script("Number.POSITIVE_INFINITY + 1 === Infinity;")
            .unwrap();
        assert!(matches!(value, JsValue::Bool(true)));
    }

    #[test]
    fn date_default_value_uses_string_conversion_and_in_uses_left_key() {
        let mut i = Interpreter::new();
        let value = i
            .run_script("let d = new Date(0); d + d === d.toString() + d.toString();")
            .unwrap();
        assert!(matches!(value, JsValue::Bool(true)));
        let value = i.run_script("'x' in {x: 1};").unwrap();
        assert!(matches!(value, JsValue::Bool(true)));
    }

    #[test]
    fn compound_assignment_evaluates_member_reference_once() {
        let mut i = Interpreter::new();
        let value = i
            .run_script(
                "let calls=0; function base(){calls++; return {x:0};} base().x ||= 2; calls;",
            )
            .unwrap();
        assert_eq!(num(value), 1.0);
    }

    #[test]
    fn descriptors_and_completion_values_are_observable() {
        let mut i = Interpreter::new();
        let value = i
            .run_script(
                "let o={}; Object.defineProperty(o,'x',{get:function(){return 0;},set:undefined,enumerable:true,configurable:true}); o.x &&= 1; o.x;",
            )
            .unwrap();
        assert_eq!(num(value), 0.0);
        let value = i
            .run_script("eval('1; for (var x=true; x; x=false) { 3; }');")
            .unwrap();
        assert_eq!(num(value), 3.0);
    }

    #[test]
    fn promise_combinators_preserve_order_and_settlement_kind() {
        let mut i = Interpreter::new();
        i.run_script(
            "Promise.all([Promise.resolve(1), 2]).then(v=>console.log(v.join(','))); Promise.allSettled([Promise.resolve(3), Promise.reject(4)]).then(v=>console.log(v[0].status, v[1].status)); Promise.any([Promise.reject(1), Promise.resolve(5)]).then(v=>console.log(v));",
        )
        .unwrap();
        assert_eq!(
            &*i.output.borrow(),
            &[
                "1,2".to_string(),
                "fulfilled rejected".to_string(),
                "5".to_string()
            ]
        );
    }

    #[test]
    fn array_object_and_for_of_patterns_bind_values_defaults_and_rest() {
        let mut i = Interpreter::new();
        let value = match i.run_script(
            "let [a,b=3,...tail]=[1,undefined,4,5]; let {x,y:z=4,...rest}={x:1,y:undefined,q:2}; a+b+tail.length+x+z+Object.keys(rest).length;",
        ) {
            Ok(value) => value,
            Err(Throw(JsValue::Object(object))) => panic!("pattern error: {:?}", object.borrow().get_own("message")),
            Err(error) => panic!("pattern error: {:?}", error.0),
        };
        assert_eq!(num(value), 12.0);
        let mut j = Interpreter::new();
        let value = j
            .run_script("let total=0; for (const [a,b] of [[2,3]]) { total=a+b; } total;")
            .unwrap();
        assert_eq!(num(value), 5.0);
        let mut k = Interpreter::new();
        let value = k
            .run_script("let a=0,b=0; [a,b]=[3,4]; ({a,b}={a:5,b:6}); a+b;")
            .unwrap();
        assert_eq!(num(value), 11.0);
        let mut m = Interpreter::new();
        let value = m
            .run_script("let key='x'; let {[key]: value}={x:9}; value;")
            .unwrap();
        assert_eq!(num(value), 9.0);
        let mut n = Interpreter::new();
        let value = n
            .run_script("let a=0,b=0; [a=3,b]=[undefined,4]; ({a,b}={a:5,b:6}); a+b;")
            .unwrap();
        assert_eq!(num(value), 11.0);
        let mut p = Interpreter::new();
        let value = p
            .run_script(
                "let out=[]; for (const [a,b] of [[1,2],[3,4]]) { out.push(a+b); } out.join(',');",
            )
            .unwrap();
        assert_eq!(p.to_string(&value).unwrap(), "3,7");
        let mut q = Interpreter::new();
        let value = q
            .run_script("let out=[]; let it={n:0,next:function(){this.n++;return {value:this.n,done:this.n>2};}}; for (const x of it) out.push(x); out.join(',');")
            .unwrap();
        assert_eq!(q.to_string(&value).unwrap(), "1,2");
        let mut r = Interpreter::new();
        let value = r
            .run_script("let closed=0; let it={next:function(){throw 'stop';},return:function(){closed++;}}; try {[...it];} catch(e) {} closed;")
            .unwrap();
        assert_eq!(num(value), 1.0);
        let mut s = Interpreter::new();
        let value = s
            .run_script("let closed=0; let n=0; let it={next:function(){n++;return {value:n,done:n>3};},return:function(){closed++;}}; try { for (const x of it) { if (x===2) throw 'stop'; } } catch(e) {} closed;")
            .unwrap();
        assert_eq!(num(value), 1.0);
        let mut t = Interpreter::new();
        let value = t
            .run_script("let it={n:0,next:function(){this.n++;return {value:this.n,done:this.n>2};}}; let iterable={ [Symbol.iterator]:function(){return it;} }; [...iterable].join(',');")
            .unwrap();
        assert_eq!(t.to_string(&value).unwrap(), "1,2");
    }

    #[test]
    fn loops_and_try_catch_are_executable() {
        let mut i = Interpreter::new();
        let v = i
            .run_script(
                "let s=0; for(let n=0;n<5;n++){s+=n;} try { throw 4; } catch(e) { s+=e; } s;",
            )
            .unwrap();
        assert_eq!(num(v), 14.0);
    }

    #[test]
    fn console_output_is_captured_without_io() {
        let mut i = Interpreter::new();
        i.run_script("console.log('hello', 3);").unwrap();
        assert_eq!(&*i.output.borrow(), &["hello 3".to_string()]);
    }

    #[test]
    fn timers_run_only_when_the_host_pumps_them() {
        let mut i = Interpreter::new();
        i.run_script("setTimeout(()=>console.log('later'),0);").unwrap();
        assert!(i.output.borrow().is_empty());
        assert!(i.pump_timers().unwrap());
        assert_eq!(&*i.output.borrow(), &["later".to_string()]);
        assert!(!i.pump_timers().unwrap());
    }
}
