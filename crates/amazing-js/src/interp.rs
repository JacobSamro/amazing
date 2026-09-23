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
    delay: Duration,
    repeat: bool,
    callback: JsValue,
    args: Vec<JsValue>,
}

#[derive(Clone, Debug)]
pub struct HostResponse {
    pub url: String,
    pub status: u16,
    pub body: String,
    pub headers: Vec<(String, String)>,
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
    fn query_selector_all(&mut self, selector: &str) -> Vec<u32>;
    fn matches_selector(&self, node: u32, selector: &str) -> bool;
    fn parent_node(&self, node: u32) -> Option<u32>;
    fn get_text_content(&self, node: u32) -> Option<String>;
    fn set_text_content(&mut self, node: u32, text: &str) -> Result<(), String>;
    fn get_document_title(&self) -> String;
    fn set_document_title(&mut self, title: &str) -> Result<(), String>;
    fn fetch(&mut self, base_url: &str, input: &str) -> Result<HostResponse, String>;
    fn get_inner_html(&self, node: u32) -> Option<String>;
    fn set_inner_html(&mut self, node: u32, html: &str) -> Result<(), String>;
    fn get_attribute(&self, node: u32, name: &str) -> Option<Option<String>>;
    fn set_attribute(&mut self, node: u32, name: &str, value: &str) -> Result<(), String>;
    fn remove_attribute(&mut self, node: u32, name: &str) -> Result<(), String>;
    fn create_element(&mut self, name: &str) -> Result<u32, String>;
    fn create_text_node(&mut self, text: &str) -> Result<u32, String>;
    fn append_child(&mut self, parent: u32, child: u32) -> Result<(), String>;
    fn append_text(&mut self, parent: u32, text: &str) -> Result<(), String>;
    fn remove_node(&mut self, node: u32) -> Result<(), String>;
    fn get_style(&self, node: u32, property: &str) -> Option<String>;
    fn set_style(&mut self, node: u32, property: &str, value: &str) -> Result<(), String>;
    fn storage_get(&self, _origin: &str, _key: &str) -> Option<String> {
        None
    }
    fn storage_set(&mut self, _origin: &str, _key: &str, _value: &str) -> Result<(), String> {
        Err("localStorage is unavailable".into())
    }
    fn storage_remove(&mut self, _origin: &str, _key: &str) -> Result<(), String> {
        Ok(())
    }
    fn storage_clear(&mut self, _origin: &str) -> Result<(), String> {
        Ok(())
    }
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
    pub regexp_proto: Obj,
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
    location_url: String,
    pending_navigation: Option<String>,
    pending_history_delta: Option<i32>,
    pending_history_update: Option<(String, bool, JsValue)>,
    history_state: JsValue,
    last_event_default_prevented: bool,
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
        let regexp_proto = Obj(Rc::new(RefCell::new({
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
            regexp_proto,
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
            location_url: String::new(),
            pending_navigation: None,
            pending_history_delta: None,
            pending_history_update: None,
            history_state: JsValue::Null,
            last_event_default_prevented: false,
        };
        i.global_obj
            .borrow_mut()
            .set_own("globalThis", JsValue::Object(i.global_obj.clone()));
        let global_this = JsValue::Object(i.global_obj.clone());
        let _ = i.global_env.declare_var("globalThis", global_this);
        let window = JsValue::Object(i.global_obj.clone());
        i.global_obj.borrow_mut().set_own("window", window.clone());
        let _ = i.global_env.declare_var("window", window);
        i.install_builtins();
        i.install_regexp();
        i.install_location();
        i.install_history();
        i.install_storage();
        i
    }

    fn install_storage(&mut self) {
        let storage = self.object_value();
        if let JsValue::Object(storage) = &storage {
            storage.borrow_mut().set_own(
                "getItem",
                Self::native("getItem", native_storage_get_item, &self.function_proto),
            );
            storage.borrow_mut().set_own(
                "setItem",
                Self::native("setItem", native_storage_set_item, &self.function_proto),
            );
            storage.borrow_mut().set_own(
                "removeItem",
                Self::native(
                    "removeItem",
                    native_storage_remove_item,
                    &self.function_proto,
                ),
            );
            storage.borrow_mut().set_own(
                "clear",
                Self::native("clear", native_storage_clear, &self.function_proto),
            );
        }
        self.global_obj
            .borrow_mut()
            .set_own("localStorage", storage.clone());
        let _ = self.global_env.declare_var("localStorage", storage);
    }

    fn install_history(&mut self) {
        let mut history = JsObject::ordinary();
        history.proto = Some(self.object_proto.clone());
        history.set_own(
            "back",
            Self::native("back", native_history_back, &self.function_proto),
        );
        history.set_own(
            "forward",
            Self::native("forward", native_history_forward, &self.function_proto),
        );
        history.set_own(
            "go",
            Self::native("go", native_history_go, &self.function_proto),
        );
        history.set_own(
            "pushState",
            Self::native("pushState", native_history_push_state, &self.function_proto),
        );
        history.set_own(
            "replaceState",
            Self::native(
                "replaceState",
                native_history_replace_state,
                &self.function_proto,
            ),
        );
        history.set_own("length", JsValue::num(1.0));
        history.set_own("state", JsValue::Null);
        let history = JsValue::Object(Obj(Rc::new(RefCell::new(history))));
        self.global_obj
            .borrow_mut()
            .set_own("history", history.clone());
        let _ = self.global_env.declare_var("history", history);
    }

    fn install_location(&mut self) {
        let mut location = JsObject::ordinary();
        location.proto = Some(self.object_proto.clone());
        let href_getter = Self::native("get href", native_location_href_get, &self.function_proto);
        let href_setter = Self::native("set href", native_location_href_set, &self.function_proto);
        let href_getter = match href_getter {
            JsValue::Object(o) => o.borrow().func.clone().expect("native href getter"),
            _ => unreachable!(),
        };
        let href_setter = match href_setter {
            JsValue::Object(o) => o.borrow().func.clone().expect("native href setter"),
            _ => unreachable!(),
        };
        let _ = location.define(
            "href",
            Property {
                value: JsValue::Undefined,
                writable: false,
                enumerable: true,
                configurable: true,
                getter: Some(Box::new(href_getter)),
                setter: Some(Box::new(href_setter)),
            },
        );
        location.set_own(
            "assign",
            Self::native("assign", native_location_assign, &self.function_proto),
        );
        location.set_own(
            "replace",
            Self::native("replace", native_location_assign, &self.function_proto),
        );
        location.set_own(
            "reload",
            Self::native("reload", native_location_reload, &self.function_proto),
        );
        let location = JsValue::Object(Obj(Rc::new(RefCell::new(location))));
        self.global_obj
            .borrow_mut()
            .set_own("[[LocationObject]]", location.clone());
        let location_getter = Self::native(
            "get location",
            native_window_location_get,
            &self.function_proto,
        );
        let location_setter = Self::native(
            "set location",
            native_window_location_set,
            &self.function_proto,
        );
        let location_getter = match location_getter {
            JsValue::Object(o) => o.borrow().func.clone().expect("native location getter"),
            _ => unreachable!(),
        };
        let location_setter = match location_setter {
            JsValue::Object(o) => o.borrow().func.clone().expect("native location setter"),
            _ => unreachable!(),
        };
        let _ = self.global_obj.borrow_mut().define(
            "location",
            Property {
                value: JsValue::Undefined,
                writable: false,
                enumerable: true,
                configurable: true,
                getter: Some(Box::new(location_getter)),
                setter: Some(Box::new(location_setter)),
            },
        );
        let _ = self.global_env.declare_var("location", location);
    }

    /// Attach a live host and install the browser-facing `document` global.
    pub fn set_dom_host(&mut self, host: Rc<RefCell<dyn DomHost>>) {
        self.dom_host = Some(host);
        let window = JsValue::Object(self.global_obj.clone());
        self.global_obj
            .borrow_mut()
            .set_own("window", window.clone());
        self.global_obj
            .borrow_mut()
            .set_own("[[DomNode]]", JsValue::num(0.0));
        self.global_obj.borrow_mut().set_own(
            "addEventListener",
            Self::native(
                "addEventListener",
                native_dom_add_event_listener,
                &self.function_proto,
            ),
        );
        self.global_obj.borrow_mut().set_own(
            "removeEventListener",
            Self::native(
                "removeEventListener",
                native_dom_remove_event_listener,
                &self.function_proto,
            ),
        );
        let _ = self.global_env.declare_var("window", window);
        self.install_location();
        let document = self.make_dom_document();
        self.global_obj
            .borrow_mut()
            .set_own("document", document.clone());
        let _ = self.global_env.declare_lexical("document", document, false);
    }

    pub fn set_location_url(&mut self, url: impl Into<String>) {
        self.location_url = url.into();
    }

    pub fn set_viewport_size(&mut self, width: u32, height: u32) {
        for (name, value) in [
            ("innerWidth", JsValue::num(width as f64)),
            ("innerHeight", JsValue::num(height as f64)),
            ("devicePixelRatio", JsValue::num(1.0)),
        ] {
            self.global_obj.borrow_mut().set_own(name, value.clone());
            if self.global_env.has(name) {
                let _ = self.global_env.assign(name, value);
            } else {
                let _ = self.global_env.declare_var(name, value);
            }
        }
    }

    pub fn take_navigation(&mut self) -> Option<String> {
        self.pending_navigation.take()
    }

    pub fn take_history_delta(&mut self) -> Option<i32> {
        self.pending_history_delta.take()
    }

    pub fn take_history_update(&mut self) -> Option<(String, bool, JsValue)> {
        self.pending_history_update.take()
    }

    pub fn set_history_length(&mut self, length: usize) {
        if let Some(JsValue::Object(history)) = self.global_obj.borrow().get_own("history") {
            history
                .borrow_mut()
                .set_own("length", JsValue::num(length as f64));
        }
    }

    pub fn set_history_state(&mut self, state: JsValue) {
        self.history_state = state.clone();
        if let Some(JsValue::Object(history)) = self.global_obj.borrow().get_own("history") {
            history.borrow_mut().set_own("state", state);
        }
    }

    pub fn install_inline_handler(
        &mut self,
        node: u32,
        event_type: &str,
        body: &str,
    ) -> Result<(), Throw> {
        let source = format!("(function(event){{{body}}})");
        let callback = self.run_script(&source)?;
        if callable_function(&callback).is_none() {
            return Err(Throw::type_error("inline event handler is not callable"));
        }
        self.set_dom_handler(node, event_type, Some(callback));
        Ok(())
    }

    pub fn last_event_default_prevented(&self) -> bool {
        self.last_event_default_prevented
    }

    /// Runs promise reactions queued by a host callback. Browser integrations
    /// call this after dispatching an input event or timer task.
    pub fn drain_microtasks(&mut self) -> Result<(), Throw> {
        self.drain_jobs()
    }

    pub fn schedule_timer(&mut self, callback: JsValue, delay_ms: u64, args: Vec<JsValue>) -> u32 {
        self.schedule_timer_kind(callback, delay_ms, args, false)
    }

    pub fn schedule_interval(
        &mut self,
        callback: JsValue,
        delay_ms: u64,
        args: Vec<JsValue>,
    ) -> u32 {
        self.schedule_timer_kind(callback, delay_ms, args, true)
    }

    fn schedule_timer_kind(
        &mut self,
        callback: JsValue,
        delay_ms: u64,
        args: Vec<JsValue>,
        repeat: bool,
    ) -> u32 {
        let id = self.next_timer_id;
        self.next_timer_id = self.next_timer_id.wrapping_add(1).max(1);
        let delay = Duration::from_millis(delay_ms);
        self.timers.push(TimerJob {
            id,
            due: Instant::now() + delay,
            delay,
            repeat,
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
        self.timers.retain_mut(|timer| {
            if timer.due <= now {
                due.push((timer.callback.clone(), timer.args.clone()));
                if timer.repeat {
                    timer.due = now + timer.delay;
                    true
                } else {
                    false
                }
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
        self.last_event_default_prevented = false;
        let target = self.make_dom_element(node);
        let mut event_obj = JsObject::ordinary();
        event_obj.proto = Some(self.object_proto.clone());
        event_obj.set_own("type", JsValue::str(event_type));
        event_obj.set_own("target", target.clone());
        event_obj.set_own("currentTarget", target.clone());
        event_obj.set_own("defaultPrevented", JsValue::Bool(false));
        event_obj.set_own(
            "stopPropagation",
            Self::native(
                "stopPropagation",
                native_event_stop_propagation,
                &self.function_proto,
            ),
        );
        event_obj.set_own(
            "preventDefault",
            Self::native(
                "preventDefault",
                native_event_prevent_default,
                &self.function_proto,
            ),
        );
        let event = JsValue::Object(Obj(Rc::new(RefCell::new(event_obj))));
        let mut current = Some(node);
        while let Some(current_node) = current {
            let current_target = self.make_dom_element(current_node);
            if let JsValue::Object(event_object) = &event {
                event_object
                    .borrow_mut()
                    .set_own("currentTarget", current_target.clone());
            }
            let callbacks: Vec<JsValue> = self
                .dom_listeners
                .iter()
                .filter(|listener| listener.node == current_node && listener.event == event_type)
                .map(|listener| listener.callback.clone())
                .collect();
            for callback in callbacks {
                self.call_value(&callback, current_target.clone(), vec![event.clone()])?;
            }
            let stopped = match &event {
                JsValue::Object(event_object) => matches!(
                    event_object.borrow().get_own("cancelBubble"),
                    Some(JsValue::Bool(true))
                ),
                _ => false,
            };
            if stopped {
                break;
            }
            current = self
                .dom_host
                .as_ref()
                .and_then(|host| host.borrow().parent_node(current_node));
        }
        self.last_event_default_prevented = match &event {
            JsValue::Object(event_object) => matches!(
                event_object.borrow().get_own("defaultPrevented"),
                Some(JsValue::Bool(true))
            ),
            _ => false,
        };
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
        self.dom_listeners.retain(|listener| {
            !(listener.node == node && listener.event == event && listener.property)
        });
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
            ExprKind::PrivateName(_) => Err(Throw::type_error(
                "private names are not yet supported by the runtime",
            )),
            ExprKind::RegExp(regex) => self.make_regexp(regex.clone()),
            ExprKind::Num(n) => self.numeric(n),
            ExprKind::Str(s) => Ok(JsValue::str(s.as_str())),
            ExprKind::Bool(b) => Ok(JsValue::Bool(*b)),
            ExprKind::Null => Ok(JsValue::Null),
            ExprKind::Ident(i) => self
                .identifier_value(env, &i.name)?
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
                            self.identifier_value(env, &i.name)?
                                .map(|v| self.typeof_value(&v))
                                .unwrap_or("undefined"),
                        ));
                    }
                    ExprKind::Paren(inner) if matches!(inner.kind, ExprKind::Ident(_)) => {
                        if let ExprKind::Ident(i) = &inner.kind {
                            return Ok(JsValue::str(
                                self.identifier_value(env, &i.name)?
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

    // testharness.js exports its API by assigning properties of global_scope.
    // Unresolved lexical names must therefore consult the global object record.
    fn identifier_value(&mut self, env: &EnvHandle, name: &str) -> Result<Option<JsValue>, Throw> {
        if let Some(value) = env.resolve(name) {
            return Ok(Some(value));
        }
        let exists = self.global_obj.borrow().has_property(name);
        if exists {
            let global = JsValue::Object(self.global_obj.clone());
            self.get_prop(&global, name).map(Some)
        } else {
            Ok(None)
        }
    }

    fn resolve_reference(
        &mut self,
        target: &ast::Expr,
        env: &EnvHandle,
    ) -> Result<(Reference, JsValue), Throw> {
        match &target.kind {
            ExprKind::Ident(identifier) => {
                if let Some(value) = env.resolve(&identifier.name) {
                    Ok((Reference::Binding(identifier.name.clone()), value))
                } else if self.global_obj.borrow().has_property(&identifier.name) {
                    let global = JsValue::Object(self.global_obj.clone());
                    let value = self.get_prop(&global, &identifier.name)?;
                    Ok((Reference::Property(global, identifier.name.clone()), value))
                } else {
                    Ok((
                        Reference::Binding(identifier.name.clone()),
                        JsValue::Undefined,
                    ))
                }
            }
            ExprKind::Member { obj, prop, .. } => {
                let base = self.eval_expr(obj, env)?;
                let key = match prop {
                    MemberProp::Ident(identifier) => identifier.name.clone(),
                    MemberProp::Private(_) => {
                        return Err(Throw::type_error(
                            "private names are not yet supported by the runtime",
                        ))
                    }
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
            MemberProp::Private(_) => Err(Throw::type_error(
                "private names are not yet supported by the runtime",
            )),
            MemberProp::Computed(x) => {
                let v = self.eval_expr(x, e)?;
                self.to_property_key(&v)
            }
        }
    }
    fn prop_key(&mut self, p: &PropKey, e: &EnvHandle) -> Result<String, Throw> {
        match p {
            PropKey::Ident(i) => Ok(i.name.clone()),
            PropKey::Private(_) => Err(Throw::type_error(
                "private names are not yet supported by the runtime",
            )),
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
            let v = if p.rest {
                array_values(self, args.iter().skip(i).cloned().collect())?
            } else if let Some(v) = args.get(i) {
                v.clone()
            } else if let Some(d) = &p.default {
                self.eval_expr(d, e)?
            } else {
                JsValue::Undefined
            };
            self.bind_pattern(&p.binding, v, ast::VarKind::Let, e)?;
        }
        let argument_count = args.len();
        let a = JsValue::object(JsObject::array());
        for (i, v) in args.into_iter().enumerate() {
            self.set_prop(&a, &i.to_string(), v)?;
        }
        self.set_prop(&a, "length", JsValue::num(argument_count as f64))?;
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
            Self::native(
                "querySelector",
                native_dom_query_selector,
                &self.function_proto,
            ),
        );
        document.set_own(
            "querySelectorAll",
            Self::native(
                "querySelectorAll",
                native_dom_query_selector_all,
                &self.function_proto,
            ),
        );
        document.set_own(
            "getElementById",
            Self::native(
                "getElementById",
                native_dom_get_element_by_id,
                &self.function_proto,
            ),
        );
        document.set_own(
            "getElementsByTagName",
            Self::native(
                "getElementsByTagName",
                native_dom_get_elements_by_tag,
                &self.function_proto,
            ),
        );
        document.set_own(
            "getElementsByClassName",
            Self::native(
                "getElementsByClassName",
                native_dom_get_elements_by_class,
                &self.function_proto,
            ),
        );
        document.set_own(
            "createElement",
            Self::native(
                "createElement",
                native_dom_create_element,
                &self.function_proto,
            ),
        );
        document.set_own(
            "createTextNode",
            Self::native(
                "createTextNode",
                native_dom_create_text_node,
                &self.function_proto,
            ),
        );
        document.set_own(
            "addEventListener",
            Self::native(
                "addEventListener",
                native_dom_add_event_listener,
                &self.function_proto,
            ),
        );
        document.set_own(
            "removeEventListener",
            Self::native(
                "removeEventListener",
                native_dom_remove_event_listener,
                &self.function_proto,
            ),
        );
        let title_getter = Self::native("get title", native_dom_title_get, &self.function_proto);
        let title_setter = Self::native("set title", native_dom_title_set, &self.function_proto);
        let title_getter = match title_getter {
            JsValue::Object(o) => o.borrow().func.clone().expect("native title getter"),
            _ => unreachable!(),
        };
        let title_setter = match title_setter {
            JsValue::Object(o) => o.borrow().func.clone().expect("native title setter"),
            _ => unreachable!(),
        };
        let _ = document.define(
            "title",
            Property {
                value: JsValue::Undefined,
                writable: false,
                enumerable: true,
                configurable: true,
                getter: Some(Box::new(title_getter)),
                setter: Some(Box::new(title_setter)),
            },
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
        let inner_getter = Self::native("get innerText", native_dom_text_get, &self.function_proto);
        let inner_setter = Self::native("set innerText", native_dom_text_set, &self.function_proto);
        let inner_getter = match inner_getter {
            JsValue::Object(o) => o.borrow().func.clone().expect("native innerText getter"),
            _ => unreachable!(),
        };
        let inner_setter = match inner_setter {
            JsValue::Object(o) => o.borrow().func.clone().expect("native innerText setter"),
            _ => unreachable!(),
        };
        let _ = element.define(
            "innerText",
            Property {
                value: JsValue::Undefined,
                writable: false,
                enumerable: true,
                configurable: true,
                getter: Some(Box::new(inner_getter)),
                setter: Some(Box::new(inner_setter)),
            },
        );
        let inner_getter = Self::native(
            "get innerHTML",
            native_dom_inner_html_get,
            &self.function_proto,
        );
        let inner_setter = Self::native(
            "set innerHTML",
            native_dom_inner_html_set,
            &self.function_proto,
        );
        let inner_getter = match inner_getter {
            JsValue::Object(o) => o.borrow().func.clone().expect("native innerHTML getter"),
            _ => unreachable!(),
        };
        let inner_setter = match inner_setter {
            JsValue::Object(o) => o.borrow().func.clone().expect("native innerHTML setter"),
            _ => unreachable!(),
        };
        let _ = element.define(
            "innerHTML",
            Property {
                value: JsValue::Undefined,
                writable: false,
                enumerable: true,
                configurable: true,
                getter: Some(Box::new(inner_getter)),
                setter: Some(Box::new(inner_setter)),
            },
        );
        element.set_own(
            "getAttribute",
            Self::native(
                "getAttribute",
                native_dom_get_attribute,
                &self.function_proto,
            ),
        );
        element.set_own(
            "setAttribute",
            Self::native(
                "setAttribute",
                native_dom_set_attribute,
                &self.function_proto,
            ),
        );
        element.set_own(
            "hasAttribute",
            Self::native(
                "hasAttribute",
                native_dom_has_attribute,
                &self.function_proto,
            ),
        );
        element.set_own(
            "removeAttribute",
            Self::native(
                "removeAttribute",
                native_dom_remove_attribute,
                &self.function_proto,
            ),
        );
        element.set_own(
            "append",
            Self::native("append", native_dom_append, &self.function_proto),
        );
        element.set_own(
            "appendChild",
            Self::native("appendChild", native_dom_append_child, &self.function_proto),
        );
        element.set_own(
            "remove",
            Self::native("remove", native_dom_remove, &self.function_proto),
        );
        element.set_own(
            "matches",
            Self::native("matches", native_dom_matches, &self.function_proto),
        );
        element.set_own(
            "closest",
            Self::native("closest", native_dom_closest, &self.function_proto),
        );
        element.set_own(
            "addEventListener",
            Self::native(
                "addEventListener",
                native_dom_add_event_listener,
                &self.function_proto,
            ),
        );
        element.set_own(
            "removeEventListener",
            Self::native(
                "removeEventListener",
                native_dom_remove_event_listener,
                &self.function_proto,
            ),
        );
        element.set_own(
            "click",
            Self::native("click", native_dom_click, &self.function_proto),
        );
        let mut style = JsObject::ordinary();
        style.proto = Some(self.object_proto.clone());
        style.set_own("[[DomStyleNode]]", JsValue::num(id as f64));
        element.set_own("style", JsValue::Object(Obj(Rc::new(RefCell::new(style)))));
        let mut dataset = JsObject::ordinary();
        dataset.proto = Some(self.object_proto.clone());
        dataset.set_own("[[DomDatasetNode]]", JsValue::num(id as f64));
        element.set_own(
            "dataset",
            JsValue::Object(Obj(Rc::new(RefCell::new(dataset)))),
        );
        let mut class_list = JsObject::ordinary();
        class_list.proto = Some(self.object_proto.clone());
        class_list.set_own("[[DomClassListNode]]", JsValue::num(id as f64));
        class_list.set_own(
            "contains",
            Self::native("contains", native_class_list_contains, &self.function_proto),
        );
        class_list.set_own(
            "add",
            Self::native("add", native_class_list_add, &self.function_proto),
        );
        class_list.set_own(
            "remove",
            Self::native("remove", native_class_list_remove, &self.function_proto),
        );
        class_list.set_own(
            "toggle",
            Self::native("toggle", native_class_list_toggle, &self.function_proto),
        );
        class_list.set_own(
            "toString",
            Self::native(
                "toString",
                native_class_list_to_string,
                &self.function_proto,
            ),
        );
        element.set_own(
            "classList",
            JsValue::Object(Obj(Rc::new(RefCell::new(class_list)))),
        );
        let onclick_getter =
            Self::native("get onclick", native_dom_onclick_get, &self.function_proto);
        let onclick_setter =
            Self::native("set onclick", native_dom_onclick_set, &self.function_proto);
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

    fn dom_dataset_id(object: &Obj) -> Option<u32> {
        match object.borrow().get_own("[[DomDatasetNode]]") {
            Some(JsValue::Num(n)) if n.is_finite() && n >= 0.0 && n.fract() == 0.0 => {
                Some(n as u32)
            }
            _ => None,
        }
    }

    fn dom_class_list_id(object: &Obj) -> Option<u32> {
        match object.borrow().get_own("[[DomClassListNode]]") {
            Some(JsValue::Num(n)) if n.is_finite() && n >= 0.0 && n.fract() == 0.0 => {
                Some(n as u32)
            }
            _ => None,
        }
    }

    fn dataset_attribute_name(property: &str) -> String {
        let mut name = String::from("data-");
        for ch in property.chars() {
            if ch.is_ascii_uppercase() {
                name.push('-');
                name.push(ch.to_ascii_lowercase());
            } else {
                name.push(ch);
            }
        }
        name
    }

    fn get_prop(&mut self, v: &JsValue, k: &str) -> Result<JsValue, Throw> {
        self.get_prop_receiver(v, k, v.clone())
    }

    fn get_prop_receiver(
        &mut self,
        v: &JsValue,
        k: &str,
        receiver: JsValue,
    ) -> Result<JsValue, Throw> {
        match v {
            JsValue::Object(o) => {
                if let Some(node) = Self::dom_dataset_id(o) {
                    let name = Self::dataset_attribute_name(k);
                    return Ok(dom_host(self)?
                        .borrow()
                        .get_attribute(node, &name)
                        .flatten()
                        .map(JsValue::str)
                        .unwrap_or(JsValue::Undefined));
                }
                if let Some(node) = Self::dom_style_id(o) {
                    if let Some(value) = dom_host(self)?.borrow().get_style(node, k) {
                        return Ok(JsValue::str(value));
                    }
                }
                if let Some(node) = dom_node_id(o) {
                    if node == 0 && (k == "body" || k == "documentElement") {
                        let selector = if k == "body" { "body" } else { "html" };
                        let id = dom_host(self)?.borrow_mut().query_selector(selector);
                        return Ok(id
                            .map(|id| self.make_dom_element(id))
                            .unwrap_or(JsValue::Null));
                    }
                    if node != 0 && (k == "id" || k == "className") {
                        let name = if k == "className" { "class" } else { "id" };
                        let value = dom_host(self)?
                            .borrow()
                            .get_attribute(node, name)
                            .flatten()
                            .unwrap_or_default();
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
                let property = o.borrow().own_prop(k).cloned();
                let prototype = o.borrow().proto.clone();
                if let Some(property) = property {
                    if let Some(getter) = property.getter {
                        let function = JsValue::Object(Obj(Rc::new(RefCell::new(
                            JsObject::function(*getter),
                        ))));
                        self.call_value(&function, receiver, Vec::new())
                    } else {
                        Ok(property.value)
                    }
                } else if let Some(p) = prototype {
                    self.get_prop_receiver(&JsValue::Object(p), k, receiver)
                } else {
                    Ok(JsValue::Undefined)
                }
            }
            JsValue::Str(s) => {
                if k == "match" {
                    Ok(Self::native(
                        "match",
                        native_string_match,
                        &self.function_proto,
                    ))
                } else if k == "search" {
                    Ok(Self::native(
                        "search",
                        native_string_search,
                        &self.function_proto,
                    ))
                } else if k == "split" {
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
        if let Some(node) = Self::dom_dataset_id(&o) {
            let name = Self::dataset_attribute_name(k);
            let value = self.to_string(&x)?;
            dom_host(self)?
                .borrow_mut()
                .set_attribute(node, &name, &value)
                .map_err(Throw::type_error)?;
            return Ok(());
        }
        if let Some(node) = dom_node_id(&o) {
            if node != 0 && (k == "id" || k == "className") {
                let name = if k == "className" { "class" } else { "id" };
                let value = self.to_string(&x)?;
                dom_host(self)?
                    .borrow_mut()
                    .set_attribute(node, name, &value)
                    .map_err(Throw::type_error)?;
                return Ok(());
            }
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
            "fetch",
            Self::native("fetch", native_fetch, &self.function_proto),
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
            "setInterval",
            Self::native("setInterval", native_set_interval, &self.function_proto),
        );
        self.define_global(
            "clearInterval",
            Self::native("clearInterval", native_clear_timeout, &self.function_proto),
        );
        self.define_global(
            "requestAnimationFrame",
            Self::native(
                "requestAnimationFrame",
                native_request_animation_frame,
                &self.function_proto,
            ),
        );
        self.define_global(
            "cancelAnimationFrame",
            Self::native(
                "cancelAnimationFrame",
                native_clear_timeout,
                &self.function_proto,
            ),
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
        let function_ctor = Self::native(
            "Function",
            native_function_constructor,
            &self.function_proto,
        );
        if let JsValue::Object(object) = &function_ctor {
            object
                .borrow_mut()
                .set_own("prototype", JsValue::Object(self.function_proto.clone()));
        }
        self.define_global("Function", function_ctor);
        self.object_proto.borrow_mut().set_own(
            "hasOwnProperty",
            Self::native(
                "hasOwnProperty",
                native_has_own_property,
                &self.function_proto,
            ),
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
                "prototype",
                JsValue::Object(self.array_proto.clone()),
            );
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
            ("some", native_array_some as crate::value::NativeFn),
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
        let string_ctor = Self::native("String", native_string, &self.function_proto);
        if let JsValue::Object(object) = &string_ctor {
            let _ = object.borrow_mut().define(
                "fromCharCode",
                regexp_property(
                    Self::native(
                        "fromCharCode",
                        native_string_from_char_code,
                        &self.function_proto,
                    ),
                    true,
                ),
            );
        }
        self.define_global("String", string_ctor);
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

include!("interp_builtins.rs");
include!("interp_regexp.rs");
include!("interp_tests.rs");
