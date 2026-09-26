//! A small, panic-free tree-walking ECMAScript runtime.
//!
//! This is the Phase 6 execution milestone. It implements ordinary values,
//! objects, arrays, functions, closures, control flow, coercion and a small
//! set of ES built-ins using only `std`. Modules, generators and a
//! real asynchronous job queue remain explicit later milestones.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::ast::{self, Binding, ExprKind, ForInit, ForLeft, MemberProp, PropKey, StmtKind};
use crate::env::{EnvHandle, EnvKind};
use crate::value::{
    number_to_string, ClassElements, DerivedConstructor, Exotic, FieldKey, FunctionBody, HostCallPolicy,
    HostFunctionId, HostHandle, InstanceElement, JsFunction, JsObject, JsValue, LegacyIteratorKind,
    LegacyIteratorState, PairIteratorKind, PairIteratorState, MapKey, MapState, Obj, PendingReaction, PrivateElementKind,
    PrivateName, PromiseFate,
    PromiseState, Property, PropertyKey, CollectionIteratorKind, CollectionIteratorState,
    SequenceIteratorKind, SequenceIteratorState, StaticElement,
    SetState, Throw, WellKnownSymbol, JsSymbol,
};

#[path = "vm/mod.rs"]
pub mod vm;
#[path = "vm/eligibility.rs"]
mod vm_eligibility;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ExecutionBackend {
    #[default]
    TreeWalk,
    Bytecode,
}

#[path = "interp_bindings.rs"]
mod bindings;
pub use bindings::BindingPropertyFlags;
#[path = "interp_legacy.rs"]
mod legacy;
pub use legacy::{LegacyPlatformHooks, PairIterableHooks};

/// The embedding's per-interpreter native entry point. `new_target` is present
/// only for construction. The callable is cloned before invocation, so a nested
/// JavaScript entry never holds a borrow of its registration slot.
pub type HostNativeDispatch = dyn Fn(
    &mut Interpreter,
    HostFunctionId,
    JsValue,
    Vec<JsValue>,
    Option<JsValue>,
) -> Result<JsValue, Throw>;

pub const MAX_CALL_DEPTH: u32 = 512;

enum Flow {
    Normal,
    Break(Option<String>),
    Continue(Option<String>),
    Return(JsValue),
}

struct ForInState {
    object: Option<Obj>,
    object_was_visited: bool,
    visited_keys: HashSet<crate::value::JsString>,
    remaining_keys: VecDeque<crate::value::JsString>,
}

struct PromiseJob {
    reaction: PendingReaction,
    fate: PromiseFate,
}

enum Job {
    Promise(PromiseJob),
    Microtask(JsValue),
}

/// An interpreter-owned root for exactly one scheduled item. Tokens cannot be
/// forged by JavaScript or used with another interpreter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SchedulingToken {
    owner: u64,
    sequence: u64,
}

/// Host-owned work ordered with JavaScript microtasks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeMicrotaskKind { MutationObserver }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeMicrotaskRequest {
    pub global: u64,
    pub kind: NativeMicrotaskKind,
}

/// Ordered requests in the runtime's bounded outbox. The runtime retains all
/// JS values; these requests contain only scalar metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchedulingRequest {
    Timer { registration: u32, callback_token: SchedulingToken, delay_ms: i32, repeat: bool },
    CancelTimer { registration: u32 },
    AnimationFrame { registration: u32, callback_token: SchedulingToken },
    CancelAnimationFrame { registration: u32 },
    Microtask { job_token: SchedulingToken },
    /// Native work retained and invoked by the embedding, ordered with JS jobs.
    NativeMicrotask { global: u64, kind: NativeMicrotaskKind },
}

enum ScheduledTask {
    Job(Job),
    Timer { registration: u32, callback: JsValue, args: Vec<JsValue>, repeat: bool },
    Frame { registration: u32, callback: JsValue },
}

struct ExternalScheduling {
    outbox: VecDeque<SchedulingRequest>,
    js_capacity: usize,
    owner: u64,
    next_sequence: u64,
    next_registration: u32,
    tasks: BTreeMap<u64, ScheduledTask>,
    timers: BTreeMap<u32, u64>,
    frames: BTreeMap<u32, u64>,
}

static NEXT_SCHEDULER_OWNER: AtomicU64 = AtomicU64::new(1);
static NEXT_HOST_REALM: AtomicU64 = AtomicU64::new(1);

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
    super_constructor: Option<JsValue>,
    new_target: Option<JsValue>,
    /// The function object actually executing in this frame (`None` for the
    /// synthetic frame a field initializer/static block runs in). Lets an
    /// explicit `super()` call find its OWN class's instance elements
    /// without threading them through every call site separately.
    own_function: Option<Obj>,
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
    Property(JsValue, PropertyKey),
    Private(JsValue, PrivateName),
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
    /// Like `matches_selector`, but binds `:scope` to `scope` instead of the
    /// document root -- `Element.closest()`'s DOM algorithm matches each
    /// inclusive ancestor with `:scope` bound to the ORIGINAL context
    /// element throughout the walk. Hosts that don't override this fall back
    /// to plain `matches_selector` (`:scope` then matches nothing but the
    /// document root, the pre-existing behavior).
    fn matches_selector_scoped(&self, node: u32, selector: &str, _scope: u32) -> bool {
        self.matches_selector(node, selector)
    }
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
    /// Native work newly pending after a host operation. This must not clear
    /// the DOM's queued flag; the adapter tracks whether it was forwarded.
    fn pending_native_microtasks(&self) -> Vec<NativeMicrotaskRequest> { Vec::new() }
    /// Called only after the runtime outbox accepts the request. Mark it
    /// forwarded, while leaving the DOM queued flag until its checkpoint.
    fn acknowledge_native_microtask(&mut self, _request: NativeMicrotaskRequest) {}
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
    pub async_function_proto: Obj,
    pub generator_function_proto: Obj,
    pub async_generator_function_proto: Obj,
    pub array_proto: Obj,
    pub string_proto: Obj,
    pub number_proto: Obj,
    pub boolean_proto: Obj,
    pub bigint_proto: Obj,
    pub symbol_proto: Obj,
    pub map_proto: Obj,
    pub set_proto: Obj,
    pub date_proto: Obj,
    pub promise_proto: Obj,
    pub regexp_proto: Obj,
    /// %Error.prototype%, kept as a stored field so the `stack` accessor's
    /// setter can compare its receiver against the exact home object
    /// (SetterThatIgnoresPrototypeProperties' SameValue check) without a
    /// global-lookup round trip that user code could shadow.
    pub error_proto: Obj,
    pub output: RefCell<Vec<String>>,
    depth: u32,
    native_stack: Vec<Obj>,
    host_native_dispatch: Option<Rc<HostNativeDispatch>>,
    host_realm: u64,
    realm_origin: String,
    window_proxy: Option<Obj>,
    window_access: Option<Rc<WindowProxyAccessHook>>,
    agent_id: u64,
    realms: HashMap<u64, RealmState>,
    intrinsic_constructors: HashMap<String, (Obj, Obj)>,
    entry_realm: Option<RealmHandle>,
    realm_callers: Vec<RealmHandle>,
    host_cache: HashMap<HostHandle, Weak<RefCell<JsObject>>>,
    put_forwards: Vec<(Weak<RefCell<JsObject>>, String, String)>,
    legacy_hooks: HashMap<u64, LegacyPlatformHooks>,
    error_brands: std::collections::HashSet<u64>,
    legacy_iterator_proto: Obj,
    pair_hooks: HashMap<u64, PairIterableHooks>,
    pair_iterator_proto: Obj,
    symbol_registry: HashMap<crate::value::JsString, JsSymbol>,
    random_state: u64,
    jobs: VecDeque<Job>,
    external_scheduling: Option<ExternalScheduling>,
    completion: JsValue,
    frames: Vec<CallFrame>,
    strict_eval_context: bool,
    /// Saved `strict_eval_context` values for the VM's `EnterStrict`/
    /// `ExitStrict` instructions, which bracket a class's heritage
    /// expression and computed member keys -- always strict, per spec,
    /// even when the enclosing (sloppy) code is compiled around them.
    /// The tree walker does this with a plain local save/restore
    /// (`eval_class`); the VM has no native call frame there, so it needs
    /// an explicit stack instead.
    vm_strict_stack: Vec<bool>,
    direct_eval_call: bool,
    direct_eval_env: Option<EnvHandle>,
    in_eval: bool,
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
    /// CDP-20's minimal debugger seam (`crate::debug_hook`): if set, called
    /// before every statement. `None` by default, so hosts that never
    /// install a hook pay nothing beyond the `Option` check.
    debug_hook: Option<crate::debug_hook::SharedDebugHook>,
    current_script_id: String,
    current_script_url: String,
}

impl Interpreter {
    pub fn new() -> Self {
        let object_proto = Obj(Rc::new(RefCell::new(JsObject::ordinary())));
        let function_proto = Obj(Rc::new(RefCell::new({
            let mut o = JsObject::function(JsFunction {
                name: String::new(), params: Vec::new(),
                body: FunctionBody::Native(native_function_prototype),
                is_arrow: false, is_async: false, is_generator: false,
                env: EnvHandle::new(EnvKind::Global, None),
                home: None, proto_for_new: None,
            });
            o.proto = Some(object_proto.clone());
            o.constructible = false;
            o
        })));
        let function_subproto = || {
            let mut object = JsObject::ordinary();
            object.proto = Some(function_proto.clone());
            Obj(Rc::new(RefCell::new(object)))
        };
        let async_function_proto = function_subproto();
        let generator_function_proto = function_subproto();
        let async_generator_function_proto = function_subproto();
        let array_proto = Obj(Rc::new(RefCell::new({
            let mut o = JsObject::array();
            o.proto = Some(object_proto.clone());
            o
        })));
        let string_proto = Obj(Rc::new(RefCell::new({
            let mut o = JsObject::ordinary();
            o.proto = Some(object_proto.clone());
            o.define("[[StringData]]", Property {
                value: JsValue::str(""), writable: false, enumerable: false,
                configurable: false, getter: None, setter: None,
            }).unwrap();
            o
        })));
        let number_proto = Obj(Rc::new(RefCell::new({
            let mut o = JsObject::ordinary();
            o.proto = Some(object_proto.clone());
            o.set_own("[[NumberData]]", JsValue::num(0.0));
            o
        })));
        let boolean_proto = Obj(Rc::new(RefCell::new({
            let mut o = JsObject::ordinary();
            o.proto = Some(object_proto.clone());
            o.set_own("[[BooleanData]]", JsValue::Bool(false));
            o
        })));
        let bigint_proto = Obj(Rc::new(RefCell::new({
            let mut o = JsObject::ordinary();
            o.proto = Some(object_proto.clone());
            o
        })));
        let symbol_proto = Obj(Rc::new(RefCell::new({
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
        let error_proto = Obj(Rc::new(RefCell::new({
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
        let legacy_iterator_proto = Obj(Rc::new(RefCell::new({
            let mut o = JsObject::ordinary();
            o.proto = Some(object_proto.clone());
            o
        })));
        let pair_iterator_proto = Obj(Rc::new(RefCell::new({
            let mut o = JsObject::ordinary();
            o.proto = Some(object_proto.clone());
            o
        })));
        let host_realm = NEXT_HOST_REALM.fetch_add(1, Ordering::Relaxed);
        let mut i = Self {
            global_env,
            global_obj,
            object_proto,
            function_proto,
            async_function_proto,
            generator_function_proto,
            async_generator_function_proto,
            array_proto,
            string_proto,
            number_proto,
            boolean_proto,
            bigint_proto,
            symbol_proto,
            map_proto,
            set_proto,
            date_proto,
            promise_proto,
            regexp_proto,
            error_proto,
            output: RefCell::new(Vec::new()),
            depth: 0,
            native_stack: Vec::new(),
            host_native_dispatch: None,
            host_realm,
            realm_origin: String::new(),
            window_proxy: None,
            window_access: None,
            agent_id: host_realm,
            realms: HashMap::new(),
            intrinsic_constructors: HashMap::new(),
            entry_realm: None,
            realm_callers: Vec::new(),
            host_cache: HashMap::new(),
            put_forwards: Vec::new(),
            legacy_hooks: HashMap::new(),
            error_brands: std::collections::HashSet::new(),
            legacy_iterator_proto,
            pair_hooks: HashMap::new(),
            pair_iterator_proto,
            symbol_registry: HashMap::new(),
            random_state: host_realm ^ 0xA076_1D64_78BD_642F,
            jobs: VecDeque::new(),
            external_scheduling: None,
            completion: JsValue::Undefined,
            frames: Vec::new(),
            strict_eval_context: false,
            vm_strict_stack: Vec::new(),
            direct_eval_call: false,
            direct_eval_env: None,
            in_eval: false,
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
            debug_hook: None,
            current_script_id: String::new(),
            current_script_url: String::new(),
        };
        i.global_obj
            .borrow_mut()
            .set_own("globalThis", JsValue::Object(i.global_obj.clone()));
        let global_this = JsValue::Object(i.global_obj.clone());
        i.global_env.bind_this(global_this.clone());
        let _ = i.global_env.declare_var("globalThis", global_this);
        let window = JsValue::Object(i.global_obj.clone());
        i.global_obj.borrow_mut().set_own("window", window.clone());
        let _ = i.global_env.declare_var("window", window);
        i.global_env.set_global_object(i.global_obj.clone());
        i.stamp_realm_objects();
        i.install_builtins();
        i.install_regexp();
        i.install_location();
        i.install_history();
        i.install_storage();
        i.stamp_realm_objects();
        i.capture_intrinsic_constructors();
        i
    }

    /// Set the per-interpreter Math.random sequence for repeatable host gates.
    pub fn set_random_seed(&mut self, seed: u64) {
        self.random_state = seed;
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
        let _ = location.define(
            "href",
            Property {
                value: JsValue::Undefined,
                writable: false,
                enumerable: true,
                configurable: true,
                getter: Some(href_getter),
                setter: Some(href_setter),
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
        let _ = self.global_obj.borrow_mut().define(
            "location",
            Property {
                value: JsValue::Undefined,
                writable: false,
                enumerable: true,
                configurable: true,
                getter: Some(location_getter),
                setter: Some(location_setter),
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

    /// Installs (or, with `None`, removes) CDP-20's minimal debugger hook —
    /// see `crate::debug_hook`. Cloning the `Rc` is cheap; call this once
    /// per realm, not per script.
    pub fn set_debug_hook(&mut self, hook: Option<crate::debug_hook::SharedDebugHook>) {
        self.debug_hook = hook;
    }

    /// Tags every subsequent `on_statement` callback with `id`/`url` until
    /// the next call. The interpreter does not itself track "which script is
    /// this statement in" (a tree-walking interpreter has no script-level
    /// frame of its own), so the host must call this immediately before
    /// running a script it wants attributed correctly; an inline event
    /// handler or a timer/microtask callback keeps whatever the last
    /// top-level script set, which is an honest simplification, not real
    /// per-callback script identity (see the driver's own doc comment).
    pub fn set_current_script(&mut self, id: impl Into<String>, url: impl Into<String>) {
        self.current_script_id = id.into();
        self.current_script_url = url.into();
    }

    /// Opt into driver-owned time and checkpoints before running page code.
    /// `js_capacity` bounds JS-originated requests. One additional slot is
    /// reserved for the flag-gated MutationObserver microtask of this agent.
    pub fn new_with_external_scheduler(js_capacity: usize) -> Result<Self, Throw> {
        if js_capacity == 0 {
            return Err(Throw::range_error("scheduler JS capacity must be positive"));
        }
        let owner = NEXT_SCHEDULER_OWNER
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| Throw::range_error("scheduler owner IDs exhausted"))?;
        let mut interpreter = Self::new();
        interpreter.external_scheduling = Some(ExternalScheduling {
            outbox: VecDeque::new(), js_capacity,
            owner, next_sequence: 1, next_registration: 1,
            tasks: BTreeMap::new(), timers: BTreeMap::new(), frames: BTreeMap::new(),
        });
        Ok(interpreter)
    }

    /// Drain ordered registration/cancellation requests after an entry point.
    /// Taking requests never runs JS and does not consume callback roots.
    pub fn take_scheduling_requests(&mut self) -> Vec<SchedulingRequest> {
        self.external_scheduling.as_mut()
            .map(|external| external.outbox.drain(..).collect())
            .unwrap_or_default()
    }

    fn emit_scheduling(&mut self, request: SchedulingRequest) -> Result<(), Throw> {
        let external = self.external_scheduling.as_mut().expect("external scheduling");
        let native = matches!(request, SchedulingRequest::NativeMicrotask { .. });
        let used = external.outbox.iter().filter(|entry| {
            matches!(entry, SchedulingRequest::NativeMicrotask { .. }) == native
        }).count();
        if native && used == 1 {
            return Err(Throw::type_error("native microtask already queued for this agent"));
        }
        if !native && used >= external.js_capacity {
            return Err(Throw::type_error("scheduler JS outbox is full"));
        }
        external.outbox.push_back(request);
        Ok(())
    }

    fn enqueue_scheduled(
        &mut self,
        task: ScheduledTask,
        request: impl FnOnce(SchedulingToken) -> SchedulingRequest,
    ) -> Result<SchedulingToken, Throw> {
        let external = self.external_scheduling.as_mut().expect("external scheduling");
        let sequence = external.next_sequence;
        external.next_sequence = sequence.checked_add(1)
            .ok_or_else(|| Throw::range_error("scheduler tokens exhausted"))?;
        let token = SchedulingToken { owner: external.owner, sequence };
        external.tasks.insert(sequence, task);
        if let Err(error) = self.emit_scheduling(request(token)) {
            self.external_scheduling.as_mut().unwrap().tasks.remove(&sequence);
            return Err(error);
        }
        Ok(token)
    }

    fn next_registration(&mut self) -> Result<u32, Throw> {
        let external = self.external_scheduling.as_mut().expect("external scheduling");
        let id = external.next_registration;
        external.next_registration = id.checked_add(1)
            .ok_or_else(|| Throw::range_error("scheduler registrations exhausted"))?;
        Ok(id)
    }

    fn enqueue_job(&mut self, job: Job) -> Result<(), Throw> {
        if self.external_scheduling.is_some() {
            self.enqueue_scheduled(ScheduledTask::Job(job), |job_token| {
                SchedulingRequest::Microtask { job_token }
            })?;
        } else {
            self.jobs.push_back(job);
        }
        Ok(())
    }

    fn enqueue_promise_job(&mut self, job: PromiseJob) -> Result<(), Throw> {
        self.enqueue_job(Job::Promise(job))
    }

    /// Append a host-owned microtask at the exact native enqueue point.
    /// Its payload stays with the host; the driver invokes the host's delivery.
    pub fn enqueue_native_microtask(&mut self, global: u64, kind: NativeMicrotaskKind) -> Result<(), Throw> {
        if self.external_scheduling.is_some() {
            self.emit_scheduling(SchedulingRequest::NativeMicrotask { global, kind })
        } else {
            Err(Throw::type_error("external scheduler is not enabled"))
        }
    }

    fn flush_native_microtasks(&mut self) -> Result<(), Throw> {
        if self.external_scheduling.is_none() { return Ok(()); }
        let Some(host) = self.dom_host.as_ref().cloned() else { return Ok(()) };
        let pending = host.borrow().pending_native_microtasks();
        for request in pending {
            self.enqueue_native_microtask(request.global, request.kind)?;
            host.borrow_mut().acknowledge_native_microtask(request);
        }
        Ok(())
    }

    /// Queue one callback in the same ordered channel as Promise reactions.
    pub fn queue_microtask(&mut self, callback: JsValue) -> Result<(), Throw> {
        if callable_function(&callback).is_none() {
            return Err(Throw::type_error("queueMicrotask callback is not callable"));
        }
        self.enqueue_job(Job::Microtask(callback))
    }

    /// Run exactly one owned Promise or queueMicrotask job. No private drain.
    pub fn invoke_one_job(&mut self, token: SchedulingToken) -> Result<(), Throw> {
        let external = self.external_scheduling.as_mut()
            .ok_or_else(|| Throw::type_error("external scheduler is not enabled"))?;
        if token.owner != external.owner || !matches!(external.tasks.get(&token.sequence), Some(ScheduledTask::Job(_))) {
            return Err(Throw::type_error("unknown or wrong-kind scheduling token"));
        }
        let Some(ScheduledTask::Job(job)) = external.tasks.remove(&token.sequence) else { unreachable!() };
        self.run_job(job)
    }

    /// Invoke one timer or animation frame. `Some(timestamp)` is required for
    /// frames and forbidden for timers; the embedding supplies the receiver.
    pub fn invoke_one_callback(&mut self, token: SchedulingToken, receiver: JsValue,
        frame_timestamp: Option<Duration>) -> Result<(), Throw> {
        let external = self.external_scheduling.as_mut()
            .ok_or_else(|| Throw::type_error("external scheduler is not enabled"))?;
        if token.owner != external.owner {
            return Err(Throw::type_error("foreign scheduling token"));
        }
        let task = external.tasks.get(&token.sequence);
        let valid = matches!((task, frame_timestamp),
            (Some(ScheduledTask::Timer { .. }), None) | (Some(ScheduledTask::Frame { .. }), Some(_)));
        if !valid { return Err(Throw::type_error("unknown or wrong-kind scheduling token")); }
        let (callback, args) = match task.unwrap() {
            ScheduledTask::Timer { registration, callback, args, repeat } => {
                let result = (callback.clone(), args.clone());
                if !repeat {
                    let registration = *registration;
                    external.tasks.remove(&token.sequence);
                    external.timers.remove(&registration);
                }
                result
            }
            ScheduledTask::Frame { callback, registration } => {
                let result = (callback.clone(), vec![JsValue::num(frame_timestamp.unwrap().as_secs_f64() * 1000.0)]);
                external.frames.remove(registration);
                external.tasks.remove(&token.sequence);
                result
            }
            ScheduledTask::Job(_) => unreachable!(),
        };
        self.call_value(&callback, receiver, args).map(|_| ())
    }

    /// Runs promise reactions queued by a host callback. Browser integrations
    /// call this after dispatching an input event or timer task.
    pub fn drain_microtasks(&mut self) -> Result<(), Throw> {
        if self.external_scheduling.is_some() {
            return Err(Throw::type_error("external scheduler owns microtasks"));
        }
        self.drain_jobs()
    }

    pub fn schedule_timer(&mut self, callback: JsValue, delay_ms: i32, args: Vec<JsValue>) -> Result<u32, Throw> {
        self.schedule_timer_kind(callback, delay_ms, args, false)
    }

    pub fn schedule_interval(
        &mut self,
        callback: JsValue,
        delay_ms: i32,
        args: Vec<JsValue>,
    ) -> Result<u32, Throw> {
        self.schedule_timer_kind(callback, delay_ms, args, true)
    }

    fn schedule_timer_kind(
        &mut self,
        callback: JsValue,
        delay_ms: i32,
        args: Vec<JsValue>,
        repeat: bool,
    ) -> Result<u32, Throw> {
        if self.external_scheduling.is_some() {
            let registration = self.next_registration()?;
            let token = self.enqueue_scheduled(
                ScheduledTask::Timer { registration, callback, args, repeat },
                |callback_token| SchedulingRequest::Timer {
                    registration, callback_token, delay_ms, repeat,
                },
            )?;
            self.external_scheduling.as_mut().unwrap().timers.insert(registration, token.sequence);
            return Ok(registration);
        }
        let id = self.next_timer_id;
        self.next_timer_id = self.next_timer_id.wrapping_add(1).max(1);
        let delay = Duration::from_millis(delay_ms.max(0) as u64);
        self.timers.push(TimerJob {
            id,
            due: Instant::now() + delay,
            delay,
            repeat,
            callback,
            args,
        });
        Ok(id)
    }

    pub fn cancel_timer(&mut self, id: u32) -> Result<(), Throw> {
        if let Some(external) = self.external_scheduling.as_ref() {
            if let Some(&sequence) = external.timers.get(&id) {
                self.emit_scheduling(SchedulingRequest::CancelTimer { registration: id })?;
                let external = self.external_scheduling.as_mut().unwrap();
                external.timers.remove(&id);
                external.tasks.remove(&sequence);
            }
            return Ok(());
        }
        self.timers.retain(|timer| timer.id != id);
        Ok(())
    }

    pub fn schedule_animation_frame(&mut self, callback: JsValue) -> Result<u32, Throw> {
        if self.external_scheduling.is_none() {
            return self.schedule_timer(callback, 16, vec![JsValue::num(0.0)]);
        }
        let registration = self.next_registration()?;
        let token = self.enqueue_scheduled(ScheduledTask::Frame { registration, callback },
            |callback_token| SchedulingRequest::AnimationFrame { registration, callback_token })?;
        self.external_scheduling.as_mut().unwrap().frames.insert(registration, token.sequence);
        Ok(registration)
    }

    pub fn cancel_animation_frame(&mut self, id: u32) -> Result<(), Throw> {
        if let Some(external) = self.external_scheduling.as_ref() {
            if let Some(&sequence) = external.frames.get(&id) {
                self.emit_scheduling(SchedulingRequest::CancelAnimationFrame { registration: id })?;
                let external = self.external_scheduling.as_mut().unwrap();
                external.frames.remove(&id);
                external.tasks.remove(&sequence);
            }
            return Ok(());
        }
        self.cancel_timer(id)
    }

    /// Executes all timers whose deadline has passed, then drains resulting
    /// Promise reactions. Returns whether at least one timer ran.
    pub fn pump_timers(&mut self) -> Result<bool, Throw> {
        if self.external_scheduling.is_some() {
            return Err(Throw::type_error("external scheduler owns timers"));
        }
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
        if self.external_scheduling.is_none() { self.drain_jobs()?; }
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
        let p = crate::parse_script(source).map_err(|e| self.normalize_throw(Throw::syntax_error(e.to_string())))?;
        self.run_program(&p)
    }

    pub fn run_script_with_backend(&mut self, source: &str, backend: ExecutionBackend) -> Result<JsValue, Throw> {
        let p = crate::parse_script(source).map_err(|e| self.normalize_throw(Throw::syntax_error(e.to_string())))?;
        self.run_program_with_backend(&p, backend)
    }

    pub fn run_program_with_backend(&mut self, p: &ast::Program, backend: ExecutionBackend) -> Result<JsValue, Throw> {
        (match backend {
            ExecutionBackend::TreeWalk => self.run_program(p),
            ExecutionBackend::Bytecode => {
                let is_module = !self.in_eval && self.global_env.kind() == EnvKind::Module;
                let code = vm::compile(p, is_module).map_err(|error| Throw::type_error(error.to_string()))?;
                let previous_entry = self.entry_realm;
                let realm = self.binding_current_realm();
                self.entry_realm.get_or_insert(realm);
                let previous_strict = self.strict_eval_context;
                self.strict_eval_context |= has_use_strict_directive(&p.body)
                    || (!self.in_eval && self.global_env.kind() == EnvKind::Module);
                let result = code.run(self, vm::DEFAULT_INSTRUCTION_LIMIT);
                self.strict_eval_context = previous_strict;
                self.entry_realm = previous_entry;
                result
            },
        }).map_err(|th| self.normalize_throw(th))
    }

    pub fn run_program(&mut self, p: &ast::Program) -> Result<JsValue, Throw> {
        let previous_entry = self.entry_realm;
        let realm = self.binding_current_realm();
        self.entry_realm.get_or_insert(realm);
        let previous_strict = self.strict_eval_context;
        if has_use_strict_directive(&p.body)
            || (!self.in_eval && self.global_env.kind() == EnvKind::Module) {
            self.strict_eval_context = true;
        }
        let result = self.run_program_inner(p).map_err(|th| self.normalize_throw(th));
        self.strict_eval_context = previous_strict;
        self.entry_realm = previous_entry;
        result
    }

    fn run_program_inner(&mut self, p: &ast::Program) -> Result<JsValue, Throw> {
        let direct_eval_env = self.direct_eval_env.take();
        if direct_eval_env.is_none() && vm_eligibility::eligible(p)
            && std::env::var_os("AMAZING_JS_VM").is_some() {
            let is_module = !self.in_eval && self.global_env.kind() == EnvKind::Module;
            match vm::compile(p, is_module) {
                Ok(code) => {
                    if std::env::var_os("AMAZING_JS_VM_TRACE").is_some() { eprintln!("AMAZING_VM compiled"); }
                    return code.run(self, vm::DEFAULT_INSTRUCTION_LIMIT);
                }
                Err(_) => {
                    if std::env::var_os("AMAZING_JS_VM_TRACE").is_some() { eprintln!("AMAZING_VM fallback"); }
                }
            }
        }
        let previous_completion = self.completion.clone();
        self.completion = JsValue::Undefined;
        let env = direct_eval_env.unwrap_or_else(|| self.global_env.clone());
        if self.in_eval { self.validate_eval_global_declarations(&p.body, &env)?; }
        self.hoist(&p.body, &env)?;
        self.in_eval = false;
        for s in &p.body {
            if let StmtKind::Expression(e) = &s.kind {
                // Bypasses `exec_stmt` for perf (no `Flow` unwrapping needed
                // for a plain top-level expression statement), so CDP-20's
                // debug hook is called explicitly here too — otherwise every
                // top-level `expr;` statement (extremely common: every bare
                // `Runtime.evaluate` call is exactly this) would be
                // invisible to a line breakpoint.
                if let Some(hook) = self.debug_hook.clone() {
                    hook.borrow_mut().on_statement(&self.current_script_id, &self.current_script_url, s.span.start.line, self.depth);
                }
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
        if self.external_scheduling.is_none() { self.drain_jobs()?; }
        let result = self.completion.clone();
        self.completion = previous_completion;
        Ok(result)
    }

    /// Module declaration instantiation hoists functions before dependency
    /// evaluation. The module linker owns the environment and import cells.
    pub fn prepare_module(&mut self, program: &ast::Program) -> Result<(), Throw> {
        let env = self.global_env.clone();
        env.0.borrow_mut().kind = EnvKind::Module;
        let previous_strict = self.strict_eval_context;
        self.strict_eval_context = true;
        let result = self.hoist(&program.body, &env);
        self.strict_eval_context = previous_strict;
        result
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
                        if self.in_eval { self.create_eval_binding(env, &n.name, Some(v))?; }
                        else { env.declare_var(&n.name, v)?; }
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
                StmtKind::ForIn { left, body, .. } | StmtKind::ForOf { left, body, .. }
                | StmtKind::ForAwaitOf { left, body, .. } => {
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
            Binding::Ident(identifier) => {
                if self.in_eval {
                    self.create_eval_binding(env, &identifier.name, None)?;
                } else {
                    env.hoist_var(&identifier.name)?;
                }
            }
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



    fn object_rest(&mut self, source: &JsValue, used: &[PropertyKey]) -> Result<JsValue, Throw> {
        let rest = self.object_value();
        for key in self.enumerable_property_keys(source)? {
            if !used.iter().any(|used_key| used_key == &key) {
                let value = self.get_prop(source, &key)?;
                self.set_prop(&rest, &key, value)?;
            }
        }
        Ok(rest)
    }

    fn exec_stmt(&mut self, s: &ast::Stmt, env: &EnvHandle) -> Result<Flow, Throw> {
        if let Some(hook) = self.debug_hook.clone() {
            hook.borrow_mut().on_statement(&self.current_script_id, &self.current_script_url, s.span.start.line, self.depth);
        }
        match &s.kind {
            StmtKind::Empty | StmtKind::Debugger => Ok(Flow::Normal),
            StmtKind::Expression(e) => {
                self.completion = self.eval_expr(e, env)?;
                Ok(Flow::Normal)
            }
            StmtKind::Var(d) => {
                for x in &d.decls {
                    if d.kind == ast::VarKind::Var && x.init.is_none() {
                        continue;
                    }
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
            StmtKind::With { .. } => Err(Throw::type_error("with statement is not implemented")),
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
                let object = match v {
                    JsValue::Null | JsValue::Undefined => None,
                    other => match array_receiver(self, other)? {
                        JsValue::Object(object) => Some(object),
                        _ => None,
                    },
                };
                let mut iterator = ForInState {
                    object,
                    object_was_visited: false,
                    visited_keys: HashSet::new(),
                    remaining_keys: VecDeque::new(),
                };
                while let Some(k) = self.next_for_in_key(&mut iterator)? {
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
            StmtKind::ForAwaitOf { .. } => Err(Throw::type_error("async iteration is not implemented")),
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
        if self.depth != 0 { return Ok(()); }
        while let Some(job) = self.jobs.pop_front() {
            self.run_job(job)?;
        }
        Ok(())
    }

    fn run_job(&mut self, job: Job) -> Result<(), Throw> {
        match job {
            Job::Promise(job) => run_promise_reaction(job.reaction, job.fate, self),
            Job::Microtask(callback) => {
                self.call_value(&callback, JsValue::Undefined, Vec::new())?;
                Ok(())
            }
        }
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
                let th = self.normalize_throw(th);
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
            ExprKind::BigInt(digits) => BinaryBigInt::parse_radix(digits, 10)
                .map(Self::bigint_value)
                .ok_or_else(|| Throw::syntax_error("invalid BigInt literal")),
            ExprKind::TaggedTemplate { .. } => Err(Throw::type_error("tagged templates are not implemented")),
            ExprKind::Str(s) => Ok(match &e.string_utf16 {
                Some(units) => JsValue::str_units(units.clone()),
                None => JsValue::str(s.as_str()),
            }),
            ExprKind::Bool(b) => Ok(JsValue::Bool(*b)),
            ExprKind::Null => Ok(JsValue::Null),
            ExprKind::Ident(i) => self
                .identifier_value(env, &i.name)?
                .ok_or_else(|| Throw::reference_error(format!("{} is not defined", i.name))),
            ExprKind::This => env.resolve_this_checked(),
            ExprKind::NewTarget => Ok(self.current_new_target().unwrap_or(JsValue::Undefined)),
            ExprKind::DynamicImport(_) => Err(Throw::type_error("dynamic import is not implemented")),
            ExprKind::DynamicImportPhase { .. } => Err(Throw::type_error("dynamic import phase is not implemented")),
            ExprKind::ImportMeta => env.resolve("\0import.meta")
                .ok_or_else(|| Throw::type_error("import.meta is only available in modules")),
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
                let mut s = Vec::<u16>::new();
                for (i, q) in t.quasis.iter().enumerate() {
                    if let Some(units) = t.quasis_utf16.get(i) {
                        s.extend_from_slice(units);
                    } else {
                        s.extend(q.encode_utf16());
                    }
                    if let Some(x) = t.exprs.get(i) {
                        let v = self.eval_expr(x, env)?;
                        s.extend_from_slice(self.to_js_string(&v)?.units());
                    }
                }
                Ok(JsValue::str_units(s))
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
                            for k in self.enumerable_property_keys(&src)? {
                                let v = self.get_prop(&src, &k)?;
                                self.set_prop(&o, &k, v)?;
                            }
                        }
                        ast::ObjElem::Prop(p) => {
                            let k = match &p.key_utf16 {
                                Some(units) => PropertyKey::String(crate::value::JsString::from_units(units.clone())),
                                None => self.prop_key(&p.key, env)?,
                            };
                            if let ast::PropValue::Method(method) = &p.value {
                                if matches!(method.kind, ast::MethodKind::Get | ast::MethodKind::Set) {
                                    let function_value = self.method_value(method, env, false);
                                    let JsValue::Object(object) = &o else { unreachable!() };
                                    let mut property = object.borrow().own_prop(&k).cloned()
                                        .filter(|property| property.getter.is_some() || property.setter.is_some())
                                        .unwrap_or(Property {
                                            value: JsValue::Undefined, writable: false,
                                            enumerable: true, configurable: true,
                                            getter: None, setter: None,
                                        });
                                    if method.kind == ast::MethodKind::Get { property.getter = Some(function_value); }
                                    else { property.setter = Some(function_value); }
                                    object.borrow_mut().define(&k, property)
                                        .map_err(|_| Throw::type_error("could not define object accessor"))?;
                                    continue;
                                }
                            }
                            let v = match &p.value {
                                ast::PropValue::Shorthand(i) => env
                                    .resolve(&i.name)
                                    .ok_or_else(|| Throw::reference_error(i.name.clone()))?,
                                ast::PropValue::Init(x) => self.eval_expr(x, env)?,
                                ast::PropValue::CoverInitializedName { .. } => {
                                    return Err(Throw::syntax_error("cover initialized name outside assignment pattern"));
                                }
                                ast::PropValue::Method(m) => self.method_value(m, env, false),
                            };
                            // A non-computed colon definition of __proto__ sets
                            // [[Prototype]] rather than creating an own property.
                            if matches!(&p.value, ast::PropValue::Init(_))
                                && !matches!(&p.key, PropKey::Computed(_))
                                && k.as_string().is_some_and(|name| *name == "__proto__")
                            {
                                let JsValue::Object(object) = &o else { unreachable!() };
                                match v {
                                    JsValue::Object(prototype) => object.borrow_mut().proto = Some(prototype),
                                    JsValue::Null => object.borrow_mut().proto = None,
                                    _ => {}
                                }
                                continue;
                            }
                            // Object-literal PropertyDefinition uses CreateDataProperty,
                            // which replaces a preceding accessor of the same name.
                            let JsValue::Object(object) = &o else { unreachable!() };
                            object.borrow_mut().define(&k, Property::data(v))
                                .map_err(|_| Throw::type_error("could not define object property"))?;
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
                if *op == ast::BinOp::Add {
                    return self.eval_add_chain(e, env);
                }
                if *op == ast::BinOp::In {
                    if let ExprKind::PrivateName(identifier) = &left.kind {
                        let name = env.resolve_private(&identifier.name).ok_or_else(|| {
                            Throw::syntax_error(format!("private name #{} is not defined", identifier.name))
                        })?;
                        let base = self.eval_expr(right, env)?;
                        return private_field_has(&base, &name).map(JsValue::Bool);
                    }
                }
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
                } else if let MemberProp::Private(identifier) = prop {
                    let name = env.resolve_private(&identifier.name).ok_or_else(|| {
                        Throw::syntax_error(format!("private name #{} is not defined", identifier.name))
                    })?;
                    private_field_get(self, &o, &name)
                } else {
                    let k = self.member_key(prop, env)?;
                    self.get_prop(&o, &k)
                }
            }
            ExprKind::Yield { .. } => Err(Throw::type_error("generators are not implemented")),
        }
    }

    fn eval_add_chain(&mut self, expression: &ast::Expr, env: &EnvHandle) -> Result<JsValue, Throw> {
        let mut rights = Vec::new();
        let mut leftmost = expression;
        while let ExprKind::Binary { op: ast::BinOp::Add, left, right } = &leftmost.kind {
            rights.push(right.as_ref());
            leftmost = left;
        }
        let mut value = self.eval_expr(leftmost, env)?;
        while let Some(right) = rights.pop() {
            let next = self.eval_expr(right, env)?;
            value = self.binary(ast::BinOp::Add, value, next)?;
        }
        Ok(value)
    }

    fn numeric(&self, n: &crate::tokens::Numeric) -> Result<JsValue, Throw> {
        if n.bigint {
            let digits: String = n.raw.chars().filter(|character| *character != '_').collect();
            let value = BinaryBigInt::parse_radix(&digits, n.radix as u32)
                .ok_or_else(|| Throw::syntax_error("invalid BigInt literal"))?;
            return Ok(Self::bigint_value(value));
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
                let value = self.eval_expr(x, env)?;
                self.unary_value(op, value)
            }
            ast::UnOp::Delete => match &x.kind {
                ExprKind::Member { obj, prop, .. } if matches!(prop, MemberProp::Private(_)) => {
                    // A private member's `delete` target is an early SyntaxError
                    // (§13.5.1.1); the parser does not reject it yet, so guard here.
                    let _ = self.eval_expr(obj, env)?;
                    Err(Throw::syntax_error("private members cannot be deleted"))
                }
                ExprKind::Member { obj, prop, .. } => {
                    let o = self.eval_expr(obj, env)?;
                    let k = self.member_key(prop, env)?;
                    if let JsValue::Object(o) = o {
                        if let Some(result) = self.window_proxy_delete(&o, &k) {
                            let deleted = result?;
                            if !deleted && self.strict_eval_context { return Err(Throw::type_error("cannot delete WindowProxy property")); }
                            return Ok(JsValue::Bool(deleted));
                        }
                        if let Some(result) = self.proxy_delete(&o, &k) {
                            let deleted = result?;
                            if !deleted && self.strict_eval_context { return Err(Throw::type_error("cannot delete proxy property")); }
                            return Ok(JsValue::Bool(deleted));
                        }
                        if matches!(o.borrow().exotic, Exotic::ModuleNamespace(_)) {
                            if o.borrow().own_prop(&k).is_some() {
                                return Err(Throw::type_error("module namespace export cannot be deleted"));
                            }
                            return Ok(JsValue::Bool(true));
                        }
                        if let Some(deleted) = binary_delete_index(&o, &k) {
                            if !deleted && self.strict_eval_context { return Err(Throw::type_error("cannot delete typed array index")); }
                            return Ok(JsValue::Bool(deleted));
                        }
                        if let Some(deleted) = match k.as_string() {
                            Some(name) => self.legacy_delete(&o, name)?,
                            None => None,
                        } {
                            Ok(JsValue::Bool(deleted))
                        } else {
                            let deleted = o.borrow_mut().delete_own(&k);
                            if !deleted && self.strict_eval_context { return Err(Throw::type_error("cannot delete non-configurable property")); }
                            Ok(JsValue::Bool(deleted))
                        }
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
            ast::UnOp::Not => {
                let value = self.eval_expr(x, env)?;
                self.unary_value(op, value)
            }
            ast::UnOp::BitNot => {
                let v = self.eval_expr(x, env)?;
                self.unary_value(op, v)
            }
            ast::UnOp::Neg => {
                let v = self.eval_expr(x, env)?;
                self.unary_value(op, v)
            }
            ast::UnOp::Pos => {
                let v = self.eval_expr(x, env)?;
                self.unary_value(op, v)
            }
        }
    }
    fn unary_value(&mut self, op: ast::UnOp, value: JsValue) -> Result<JsValue, Throw> {
        match op {
            ast::UnOp::Neg => {
                let primitive = self.primitive(value)?;
                if let JsValue::BigInt(value) = &primitive { Ok(Self::bigint_value(value.negated())) }
                else { Ok(JsValue::num(-self.to_number(&primitive)?)) }
            },
            ast::UnOp::Pos => Ok(JsValue::num(self.to_number(&value)?)),
            ast::UnOp::Not => Ok(JsValue::Bool(!value.to_boolean())),
            ast::UnOp::BitNot => {
                let primitive = self.primitive(value)?;
                if let JsValue::BigInt(value) = &primitive { Ok(Self::bigint_value(value.bit_not())) }
                else { Ok(JsValue::num((!self.to_int32(&primitive)?) as f64)) }
            },
            ast::UnOp::Typeof => Ok(JsValue::str(self.typeof_value(&value))),
            ast::UnOp::Void => Ok(JsValue::Undefined),
            ast::UnOp::Delete | ast::UnOp::Await => Err(Throw::type_error("invalid unary value operation")),
        }
    }
    fn update_value(&mut self, op: ast::UpdateOp, old: JsValue) -> Result<JsValue, Throw> {
        let primitive = self.primitive(old)?;
        if let JsValue::BigInt(value) = primitive {
            Ok(Self::bigint_value(if op == ast::UpdateOp::Inc { value.add(&BinaryBigInt::one()) }
                else { value.sub(&BinaryBigInt::one()) }))
        } else {
            let delta = if op == ast::UpdateOp::Inc { 1.0 } else { -1.0 };
            Ok(JsValue::num(self.to_number(&primitive)? + delta))
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
        let next = self.update_value(op, old.clone())?;
        self.put_reference(reference, next.clone(), env)?;
        Ok(if prefix { next } else { old })
    }
    fn binary(&mut self, op: ast::BinOp, a: JsValue, b: JsValue) -> Result<JsValue, Throw> {
        use ast::BinOp::*;
        match op {
            Add => {
                let x = self.primitive(a)?;
                let y = self.primitive(b)?;
                if x.is_str() || y.is_str() {
                    let left = self.to_js_string(&x)?;
                    let right = self.to_js_string(&y)?;
                    let mut units = Vec::with_capacity(left.len() + right.len());
                    units.extend_from_slice(left.units());
                    units.extend_from_slice(right.units());
                    Ok(JsValue::str_units(units))
                } else if matches!(x, JsValue::BigInt(_)) || matches!(y, JsValue::BigInt(_)) {
                    self.bigint_operator(op, x, y)
                } else {
                    Ok(JsValue::num(self.to_number(&x)? + self.to_number(&y)?))
                }
            }
            Sub | Mul | Div | Rem | Pow | Shl | Shr | UShr | BitAnd | BitOr | BitXor => {
                let x = self.primitive(a)?;
                let y = self.primitive(b)?;
                if matches!(x, JsValue::BigInt(_)) || matches!(y, JsValue::BigInt(_)) {
                    return self.bigint_operator(op, x, y);
                }
                Ok(JsValue::num(match op {
                    Sub => self.to_number(&x)? - self.to_number(&y)?,
                    Mul => self.to_number(&x)? * self.to_number(&y)?,
                    Div => self.to_number(&x)? / self.to_number(&y)?,
                    Rem => self.to_number(&x)? % self.to_number(&y)?,
                    Pow => self.to_number(&x)?.powf(self.to_number(&y)?),
                    Shl | Shr | UShr => {
                        let left = self.to_int32(&x)?;
                        let shift = self.to_uint32(&y)? & 31;
                        match op { Shl => left.wrapping_shl(shift) as f64,
                            Shr => (left >> shift) as f64,
                            _ => (left as u32).wrapping_shr(shift) as f64 }
                    }
                    BitAnd | BitOr | BitXor => {
                        let left = self.to_int32(&x)?;
                        let right = self.to_int32(&y)?;
                        match op { BitAnd => (left & right) as f64,
                            BitOr => (left | right) as f64,
                            _ => (left ^ right) as f64 }
                    }
                    _ => unreachable!(),
                }))
            }
            Lt | Gt | Le | Ge => {
                let x = self.primitive(a)?;
                let y = self.primitive(b)?;
                let c = if x.is_str() && y.is_str() {
                    Some(x.as_str().unwrap().cmp(y.as_str().unwrap()))
                } else if matches!(x, JsValue::BigInt(_)) || matches!(y, JsValue::BigInt(_)) {
                    self.compare_bigint_primitives(&x, &y)?
                } else {
                    self.to_number(&x)?
                        .partial_cmp(&self.to_number(&y)?)
                };
                Ok(JsValue::Bool(match op {
                    Lt => c.is_some_and(|order| order.is_lt()),
                    Gt => c.is_some_and(|order| order.is_gt()),
                    Le => c.is_some_and(|order| order.is_le()),
                    Ge => c.is_some_and(|order| order.is_ge()),
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
                    JsValue::Object(o) => {
                        if let Some(result) = self.window_proxy_has(&o, &k) { return Ok(JsValue::Bool(result?)); }
                        if let Some(result) = self.proxy_has(&o, &k) { return Ok(JsValue::Bool(result?)); }
                        Ok(JsValue::Bool(match k.as_string() {
                        _ if binary_index_descriptor(&o, &k).is_some() => binary_index_descriptor(&o, &k).flatten().is_some(),
                        Some(name) => self.legacy_has_property(&o, name)?,
                        None => o.borrow().has_property(&k),
                    })) },
                    _ => Err(Throw::type_error("right operand of in is not an object")),
                }
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
                    object.define("name", Property {
                        value: JsValue::str(name), writable: false, enumerable: false,
                        configurable: true, getter: None, setter: None,
                    }).unwrap();
                }
            }
        }
    }

    // testharness.js exports its API by assigning properties of global_scope.
    // Unresolved lexical names must therefore consult the global object record.
    fn identifier_value(&mut self, env: &EnvHandle, name: &str) -> Result<Option<JsValue>, Throw> {
        if env.is_uninitialized(name) {
            return Err(Throw::reference_error(format!("{name} is uninitialized")));
        }
        if let Some(global) = env.global_binding_object(name) {
            return self.get_prop(&JsValue::Object(global), name).map(Some);
        }
        if let Some(value) = env.resolve(name) {
            return Ok(Some(value));
        }
        let exists = self.default_has_property(&self.global_obj.clone(), &PropertyKey::from(name))?;
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
                if let Some(global) = env.global_binding_object(&identifier.name) {
                    let global = JsValue::Object(global);
                    let value = self.get_prop(&global, &identifier.name)?;
                    return Ok((Reference::Property(global, identifier.name.clone().into()), value));
                }
                if let Some(value) = env.resolve(&identifier.name) {
                    Ok((Reference::Binding(identifier.name.clone()), value))
                } else if self.default_has_property(&self.global_obj.clone(), &PropertyKey::from(identifier.name.as_str()))? {
                    let global = JsValue::Object(self.global_obj.clone());
                    let value = self.get_prop(&global, &identifier.name)?;
                    Ok((Reference::Property(global, identifier.name.clone().into()), value))
                } else {
                    Ok((
                        Reference::Binding(identifier.name.clone()),
                        JsValue::Undefined,
                    ))
                }
            }
            ExprKind::Member { obj, prop, .. } => {
                let base = self.eval_expr(obj, env)?;
                if let MemberProp::Private(identifier) = prop {
                    let name = env.resolve_private(&identifier.name).ok_or_else(|| {
                        Throw::syntax_error(format!("private name #{} is not defined", identifier.name))
                    })?;
                    let value = private_field_get(self, &base, &name)?;
                    return Ok((Reference::Private(base, name), value));
                }
                let key = match prop {
                    MemberProp::Ident(identifier) => identifier.name.clone().into(),
                    MemberProp::Private(_) => unreachable!("handled above"),
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
                if name == "location" && env.shares_binding_frame(&self.global_env, &name) {
                    let window = JsValue::Object(self.global_obj.clone());
                    return self.set_prop(&window, "location", value);
                }
                // A `var`-like global binding (declared or built in, e.g.
                // `undefined`/`NaN`/`Infinity`) is mirrored onto the global
                // object; Env::assign_raw applies the write when the mirrored
                // property is writable but otherwise silently no-ops it while
                // still reporting success, so a non-writable global never
                // throws even in strict mode. Route through the object's own
                // [[Set]] (set_with_receiver), which already has correct
                // per-property writable/accessor semantics, and apply PutValue's
                // strict/sloppy handling of an unsuccessful [[Set]] exactly as
                // Reference::Property does below.
                if let Some(global) = env.global_binding_object(&name) {
                    let key = PropertyKey::from(name.as_str());
                    let receiver = JsValue::Object(global.clone());
                    if self.set_with_receiver(&global, &key, value.clone(), receiver)? {
                        // Keep the env-level binding cell's mirror in sync;
                        // harmless no-op re-write since we just confirmed it
                        // succeeded (reads of a global non-lexical binding go
                        // through the object property directly, not this
                        // cell, but other bindings still fall back to it).
                        let _ = env.assign(&name, value);
                        return Ok(());
                    } else if !self.strict_eval_context {
                        // PutValue ignores an unsuccessful [[Set]] for sloppy references.
                        return Ok(());
                    } else {
                        return Err(Throw::type_error(format!("Cannot assign to read only property '{name}'")));
                    }
                }
                if !env.assign(&name, value.clone())? {
                    self.global_env.declare_var(&name, value)?;
                }
                Ok(())
            }
            Reference::Property(base, key) => {
                let JsValue::Object(object) = &base else {
                    return self.set_prop(&base, &key, value);
                };
                if self.set_with_receiver(object, &key, value, base.clone())? {
                    Ok(())
                } else if !self.strict_eval_context {
                    // PutValue ignores an unsuccessful [[Set]] for sloppy references.
                    Ok(())
                } else {
                    Err(Throw::type_error("cannot assign to property"))
                }
            }
            Reference::Private(base, name) => private_field_set(self, &base, &name, value),
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
                                ast::PropValue::CoverInitializedName { binding, default } => {
                                    let assigned = if matches!(&item, JsValue::Undefined) {
                                        self.eval_expr(default, env)?
                                    } else {
                                        item
                                    };
                                    let reference = ast::Expr::new(
                                        ExprKind::Ident(binding.clone()), binding.span,
                                    );
                                    self.assign_pattern_expr(&reference, &assigned, env)?;
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
        if matches!(&c.kind, ExprKind::Super) {
            let frame = self.frames.last().ok_or_else(|| Throw::syntax_error("super() used outside a constructor"))?;
            let constructor = frame.super_constructor.clone().ok_or_else(|| Throw::syntax_error("super() used outside a derived constructor"))?;
            let new_target = frame.new_target.clone().ok_or_else(|| Throw::type_error("derived constructor requires new"))?;
            let own_function = frame.own_function.clone();
            let argv = self.eval_args(args, e)?;
            let result = self.construct_with_new_target(&constructor, argv, new_target)?;
            e.initialize_this(result.clone())?;
            if let Some(frame) = self.frames.last_mut() {
                frame.this = result.clone();
            }
            if let Some(own_function) = own_function {
                if let Some(elements) = own_function.borrow().class_elements.clone() {
                    initialize_instance_elements(self, &result, &elements)?;
                }
            }
            return Ok(result);
        }
        let (f, this) = if let ExprKind::Member {
            obj,
            prop,
            optional: m,
        } = &c.kind
        {
            let o = self.eval_expr(obj, e)?;
            if (optional || *m) && (o.is_null() || o.is_undefined()) {
                return Ok(JsValue::Undefined);
            }
            if let MemberProp::Private(identifier) = prop {
                let name = e.resolve_private(&identifier.name).ok_or_else(|| {
                    Throw::syntax_error(format!("private name #{} is not defined", identifier.name))
                })?;
                let value = private_field_get(self, &o, &name)?;
                (value, o.clone())
            } else {
                let k = self.member_key(prop, e)?;
                let receiver = if matches!(&obj.kind, ExprKind::Super) {
                    e.resolve_this_checked()?
                } else {
                    o.clone()
                };
                (self.get_prop(&o, &k)?, receiver)
            }
        } else {
            (self.eval_expr(c, e)?, JsValue::Undefined)
        };
        if optional && (f.is_null() || f.is_undefined()) {
            return Ok(JsValue::Undefined);
        }
        let argv = self.eval_args(args, e)?;
        let previous_direct_eval = self.direct_eval_call;
        let previous_direct_eval_env = self.direct_eval_env.take();
        self.direct_eval_call = !optional && matches!(&c.kind, ExprKind::Ident(name) if name.name == "eval") && self.is_intrinsic_eval(&f);
        self.direct_eval_env = self.direct_eval_call.then(|| e.clone());
        let result = self.call_value(&f, this, argv);
        self.direct_eval_call = previous_direct_eval;
        self.direct_eval_env = previous_direct_eval_env;
        result
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
    fn member_key(&mut self, p: &MemberProp, e: &EnvHandle) -> Result<PropertyKey, Throw> {
        match p {
            MemberProp::Ident(i) => Ok(i.name.clone().into()),
            MemberProp::Private(_) => Err(Throw::type_error(
                "private names are not yet supported by the runtime",
            )),
            MemberProp::Computed(x) => {
                let v = self.eval_expr(x, e)?;
                self.to_property_key(&v)
            }
        }
    }
    fn prop_key(&mut self, p: &PropKey, e: &EnvHandle) -> Result<PropertyKey, Throw> {
        match p {
            PropKey::Ident(i) => Ok(i.name.clone().into()),
            PropKey::Private(_) => Err(Throw::type_error(
                "private names are not yet supported by the runtime",
            )),
            PropKey::Str(s) => Ok(s.clone().into()),
            PropKey::Num(n) => Ok(number_to_string(self.numeric(n)?.to_number()?).into()),
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
        self.call_value_with_new_target(f, this, args, None).map_err(|th| self.normalize_throw(th))
    }

    fn call_value_with_new_target_inner(
        &mut self,
        f: &JsValue,
        this: JsValue,
        args: Vec<JsValue>,
        new_target: Option<JsValue>,
        caller_realm: RealmHandle,
    ) -> Result<JsValue, Throw> {
        let o = match f {
            JsValue::Object(o) => o.clone(),
            _ => return Err(Throw::type_error("value is not callable")),
        };
        if matches!(o.borrow().exotic, Exotic::Proxy(_)) {
            return match new_target {
                Some(nt) => self.proxy_construct(&o, args, nt),
                None => self.proxy_call(&o, this, args),
            };
        }
        let fun = o
            .borrow()
            .func
            .clone()
            .ok_or_else(|| Throw::type_error("value is not callable"))?;
        let strict_function = o.borrow().strict_function;
        let interpreted = matches!(&fun.body, FunctionBody::Block(_) | FunctionBody::Expr(_) | FunctionBody::Bytecode(_));
        let this = if interpreted && !fun.is_arrow && !strict_function {
            match this {
                JsValue::Null | JsValue::Undefined => self.global_env.global_scope().resolve_this(),
                JsValue::Object(_) => this,
                primitive => native_object(self, JsValue::Undefined, vec![primitive])?,
            }
        } else { this };
        let derived_constructor = o.borrow().derived_constructor.clone();
        let lexical_super_constructor = o.borrow().lexical_super_constructor.clone();
        let lexical_new_target = o.borrow().lexical_new_target.clone();
        let async_body = self.async_body_code(&o, &fun, strict_function);
        let bytecode_async = async_body.is_some()
            || (fun.is_async && matches!(&fun.body, FunctionBody::Bytecode(_)));
        if self.depth >= MAX_CALL_DEPTH {
            return Err(Throw::range_error("maximum call stack size exceeded"));
        }
        self.depth += 1;
        self.frames.push(CallFrame {
            this: this.clone(),
            home: fun.home.clone(),
            super_constructor: derived_constructor.as_ref().map(|meta| meta.parent.clone()).or(lexical_super_constructor),
            new_target: new_target.clone().or(lexical_new_target.clone()),
            own_function: Some(o.clone()),
        });
        let previous_completion = self.completion.clone();
        let previous_strict = self.strict_eval_context;
        if interpreted { self.strict_eval_context = strict_function; }
        self.completion = JsValue::Undefined;
        let r = (|| -> Result<JsValue, Throw> {
            match fun.body {
                FunctionBody::Bytecode(code) => {
                    let generator_proto = if fun.is_generator {
                        match o.borrow().get_own("prototype") {
                            Some(JsValue::Object(proto)) => Some(proto),
                            _ => None,
                        }
                    } else {
                        None
                    };
                    code.call(
                        self, fun.env.clone(), fun.is_arrow, this, args, generator_proto,
                        derived_constructor.clone(), new_target.clone().or(lexical_new_target.clone()),
                    )
                }
                FunctionBody::Native(n) => {
                    self.native_stack.push(o.clone());
                    let result = n(self, this, args);
                    self.native_stack.pop();
                    result
                }
                FunctionBody::HostNative(id, policy, realm) => {
                    if realm != self.host_realm {
                        return Err(Throw::type_error("host function belongs to another interpreter"));
                    }
                    let constructing = new_target.is_some();
                    if (constructing && policy == HostCallPolicy::CallOnly)
                        || (!constructing && policy == HostCallPolicy::ConstructOnly)
                    {
                        return Err(Throw::type_error(if constructing {
                            "host function is not a constructor"
                        } else {
                            "host constructor requires new"
                        }));
                    }
                    let dispatch = self.host_native_dispatch.clone().ok_or_else(|| {
                        Throw::type_error("host function dispatcher is not installed")
                    })?;
                    let result = dispatch(self, id, this, args, new_target);
                    self.flush_native_microtasks()?;
                    result
                }
                FunctionBody::Expr(x) => {
                    let e = EnvHandle::new(if fun.is_arrow { EnvKind::Arrow } else { EnvKind::Function }, Some(fun.env.clone()));
                    if !fun.is_arrow { e.bind_this(this); }
                    self.bind_params(&e, &fun.params, args, fun.is_arrow)?;
                    if let Some(code) = &async_body { return code.call_async_with_env(self, e); }
                    self.eval_expr(&x, &e)
                }
                FunctionBody::Block(b) => {
                    let e = EnvHandle::new(if fun.is_arrow { EnvKind::Arrow } else { EnvKind::Function }, Some(fun.env.clone()));
                    let derived = derived_constructor.is_some();
                    if derived {
                        if new_target.is_none() {
                            return Err(Throw::type_error("derived constructor requires new"));
                        }
                        e.mark_this_uninitialized();
                        if derived_constructor.as_ref().is_some_and(|meta| meta.implicit) {
                            let parent = &derived_constructor.as_ref().expect("derived parent").parent;
                            let bound = self.construct_with_new_target(parent, args.clone(), new_target.clone().expect("newTarget"))?;
                            e.initialize_this(bound.clone())?;
                            if let Some(frame) = self.frames.last_mut() {
                                frame.this = bound.clone();
                            }
                            if let Some(elements) = o.borrow().class_elements.clone() {
                                initialize_instance_elements(self, &bound, &elements)?;
                            }
                        }
                    } else {
                        if !fun.is_arrow { e.bind_this(this.clone()); }
                        if new_target.is_some() {
                            if let Some(elements) = o.borrow().class_elements.clone() {
                                initialize_instance_elements(self, &this, &elements)?;
                            }
                        }
                    }
                    self.bind_params(&e, &fun.params, args, fun.is_arrow)?;
                    if let Some(code) = &async_body { return code.call_async_with_env(self, e); }
                    self.hoist(&b.body, &e)?;
                    let completion = self.exec_stmts(&b.body, &e)?;
                    if derived {
                        self.complete_derived_constructor(completion, &e, caller_realm)
                    } else {
                        match completion {
                            Flow::Return(value) => Ok(value),
                            _ => Ok(JsValue::Undefined),
                        }
                    }
                }
            }
        })();
        self.completion = previous_completion;
        self.strict_eval_context = previous_strict;
        self.frames.pop();
        self.depth -= 1;
        if bytecode_async {
            match r {
                Ok(value) => Ok(value),
                Err(Throw(value)) => Ok(new_promise(self, PromiseFate::Rejected(value))),
            }
        } else if fun.is_async {
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
        is_arrow: bool,
    ) -> Result<(), Throw> {
        // Parameters are mutable function bindings, not lexical declarations:
        // body var declarations reuse them and sloppy duplicate names overwrite.
        // Allocate locally before binding patterns so assignment cannot escape
        // into a same-named binding in the enclosing environment.
        let mut names = std::collections::HashSet::new();
        for p in ps {
            for name in binding_names(&p.binding) {
                if names.insert(name.clone()) {
                    e.declare_uninitialized(&name, true)?;
                }
            }
        }
        for (i, p) in ps.iter().enumerate() {
            let v = if p.rest {
                array_values(self, args.iter().skip(i).cloned().collect())?
            } else {
                args.get(i).cloned().unwrap_or(JsValue::Undefined)
            };
            let v = if v.is_undefined() {
                if let Some(default) = &p.default { self.eval_expr(default, e)? } else { v }
            } else { v };
            self.bind_pattern_mode(&p.binding, v, None, e)?;
        }
        for name in &names { e.finish_parameter_initialization(name); }
        if is_arrow || names.contains("arguments") { return Ok(()); }
        let argument_count = args.len();
        let a = self.object_value();
        for (i, v) in args.into_iter().enumerate() {
            self.set_prop(&a, &i.to_string(), v)?;
        }
        if let JsValue::Object(object) = &a {
            object.borrow_mut().define("length", Property {
                value: JsValue::num(argument_count as f64), writable: true,
                enumerable: false, configurable: true, getter: None, setter: None,
            }).unwrap();
            if let Some(values) = self.array_proto.borrow().get_own("values") {
                object.borrow_mut().define(WellKnownSymbol::Iterator.symbol(), Property {
                    value: values, writable: true, enumerable: false, configurable: true,
                    getter: None, setter: None,
                }).unwrap();
            }
        }
        e.declare_var("arguments", a)?;
        Ok(())
    }
    fn construct(&mut self, f: &JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
        self.construct_with_new_target(f, args, f.clone())
    }
    fn construct_with_new_target(&mut self, f: &JsValue, args: Vec<JsValue>, new_target: JsValue) -> Result<JsValue, Throw> {
        let o = match f {
            JsValue::Object(o) => o.clone(),
            _ => return Err(Throw::type_error("constructor is not an object")),
        };
        if matches!(o.borrow().exotic, Exotic::Proxy(_)) {
            return self.proxy_construct(&o, args, new_target);
        }
        let fun = o
            .borrow()
            .func
            .clone()
            .ok_or_else(|| Throw::type_error("not a constructor"))?;
        if matches!(&fun.body, FunctionBody::Native(native)
            if std::ptr::fn_addr_eq(*native, bigint_constructor as crate::value::NativeFn)) {
            return Err(Throw::type_error("BigInt is not a constructor"));
        }
        if fun.is_arrow {
            return Err(Throw::type_error("arrow is not a constructor"));
        }
        if !o.borrow().constructible {
            return Err(Throw::type_error("function is not a constructor"));
        }
        if Self::is_bound_function(f) {
            return self.invoke_bound_function(f, args, Some(new_target));
        }
        let derived = o.borrow().derived_constructor.is_some();
        if derived {
            let result = self.call_value_with_new_target(
                f,
                JsValue::Undefined,
                args,
                Some(new_target),
            )?;
            return if result.is_object() {
                Ok(result)
            } else {
                Err(Throw::type_error("derived constructor returned a non-object"))
            };
        }
        if !new_target.is_object() { return Err(Throw::type_error("newTarget is not an object")); }
        let proto = match self.get_prop(&new_target, "prototype")? {
            JsValue::Object(p) => p,
            _ => self.constructor_fallback_prototype(f, &new_target),
        };
        let obj = Obj(Rc::new(RefCell::new({
            let mut x = JsObject::ordinary();
            x.proto = Some(proto.clone());
            x.constructed = true;
            x
        })));
        let v = self.call_value_with_new_target(
            f,
            JsValue::Object(obj.clone()),
            args,
            Some(new_target.clone()),
        )?;
        if v.is_object() {
            // Native and host constructors may return an existing object. A
            // subclass still supplies its prototype through newTarget; in
            // particular, a canonical DOM wrapper must retain its identity.
            if matches!(fun.body, FunctionBody::Native(_) | FunctionBody::HostNative(..)) && new_target != *f {
                if let JsValue::Object(result) = &v {
                    result.borrow_mut().proto = Some(proto);
                }
            }
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
        o.proto = Some(if f.is_async && f.is_generator {
            self.async_generator_function_proto.clone()
        } else if f.is_async {
            self.async_function_proto.clone()
        } else if f.is_generator {
            self.generator_function_proto.clone()
        } else {
            self.function_proto.clone()
        });
        o.strict_function = self.strict_eval_context || has_use_strict_directive(&f.body.body);
        let h = Obj(Rc::new(RefCell::new(o)));
        let p = Obj(Rc::new(RefCell::new({
            let mut x = JsObject::ordinary();
            x.proto = Some(self.object_proto.clone());
            x
        })));
        if !f.is_generator {
            p.borrow_mut().define("constructor", symbol_data(JsValue::Object(h.clone()), true, true)).unwrap();
        }
        h.borrow_mut()
            .set_own("prototype", JsValue::Object(p.clone()));
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
        // An arrow has no `[[Prototype]]` special-case of its own: it is
        // %AsyncFunction.prototype% when async, matching `make_function`
        // and the VM's `instantiate` (arrows are never generators, so that
        // combination does not apply here). This had been hardcoded to
        // `function_proto` for every arrow, sync or async.
        o.proto = Some(if a.is_async {
            self.async_function_proto.clone()
        } else {
            self.function_proto.clone()
        });
        o.strict_function = self.strict_eval_context || matches!(&a.body, ast::ArrowBody::Block(block) if has_use_strict_directive(&block.body));
        let handle = Obj(Rc::new(RefCell::new(o)));
        if let Some(frame) = self.frames.last() {
            let mut arrow = handle.borrow_mut();
            arrow.lexical_super_constructor = frame.super_constructor.clone();
            arrow.lexical_new_target = frame.new_target.clone();
        }
        JsValue::Object(handle)
    }
    fn method_value(&self, m: &ast::ClassMethod, e: &EnvHandle, class_method: bool) -> JsValue {
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
        o.proto = Some(if m.is_async && m.is_generator {
            self.async_generator_function_proto.clone()
        } else if m.is_async {
            self.async_function_proto.clone()
        } else if m.is_generator {
            self.generator_function_proto.clone()
        } else {
            self.function_proto.clone()
        });
        o.strict_function = class_method || self.strict_eval_context || has_use_strict_directive(&m.body.body);
        let handle = Obj(Rc::new(RefCell::new(o)));
        JsValue::Object(handle)
    }
    fn eval_class(&mut self, c: &ast::ClassItem, e: &EnvHandle) -> Result<JsValue, Throw> {
        let previous_strict = self.strict_eval_context;
        self.strict_eval_context = true;
        let result = self.eval_class_inner(c, e);
        self.strict_eval_context = previous_strict;
        result
    }

    fn eval_class_inner(&mut self, c: &ast::ClassItem, e: &EnvHandle) -> Result<JsValue, Throw> {
        let ctor = c.members.iter().find_map(|m| {
            if let ast::ClassMember::Method(x) = m {
                (x.kind == ast::MethodKind::Constructor).then_some(x)
            } else {
                None
            }
        });
        // NewPrivateEnvironment (§9.2): every method, field initializer, and
        // static block of this class captures `class_env`, not the raw outer
        // scope, so `#name` inside any of them resolves through it.
        let class_env = EnvHandle::new(EnvKind::Block, Some(e.clone()));
        class_env.set_class_private(collect_private_names(&c.members));
        let f = if let Some(x) = ctor {
            self.method_value(x, &class_env, true)
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
                &class_env,
            )
        };
        if let JsValue::Object(o) = &f {
            o.borrow_mut().class_constructor = true;
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
                p.borrow_mut().define("constructor", symbol_data(f.clone(), true, true)).unwrap();
                if let Some(super_class) = &c.super_class {
                    let super_value = self.eval_expr(super_class, &class_env)?;
                    let super_prototype = match &super_value {
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
                    o.borrow_mut().proto = match &super_value {
                        JsValue::Object(parent) => Some(parent.clone()),
                        _ => unreachable!(),
                    };
                    o.borrow_mut().derived_constructor = Some(DerivedConstructor {
                        parent: super_value,
                        implicit: ctor.is_none(),
                    });
                }
                let mut class_function = o.borrow_mut();
                if let Some(function) = &mut class_function.func {
                    function.home = Some(p.clone());
                }
                drop(class_function);
                let mut instance_elements: Vec<InstanceElement> = Vec::new();
                let mut static_elements: Vec<StaticElement> = Vec::new();
                for m in &c.members {
                    match m {
                        ast::ClassMember::Method(x) if x.kind != ast::MethodKind::Constructor => {
                            let target_home = if x.is_static { o.clone() } else { p.clone() };
                            let method = self.method_value(x, &class_env, true);
                            if let JsValue::Object(method_object) = &method {
                                method_object.borrow_mut().func.as_mut().unwrap().home = Some(target_home.clone());
                            }
                            if let ast::PropKey::Private(identifier) = &x.key {
                                let name = class_env
                                    .resolve_private(&identifier.name)
                                    .expect("collect_private_names recorded every private key");
                                let is_getter = x.kind == ast::MethodKind::Get;
                                let is_accessor = matches!(x.kind, ast::MethodKind::Get | ast::MethodKind::Set);
                                if x.is_static {
                                    upsert_static_private_accessor_or_method(
                                        &mut static_elements, name, is_accessor, is_getter, method,
                                    );
                                } else {
                                    upsert_instance_private_accessor_or_method(
                                        &mut instance_elements, name, is_accessor, is_getter, method,
                                    );
                                }
                                continue;
                            }
                            let k = match &x.key_utf16 {
                                Some(units) => PropertyKey::String(crate::value::JsString::from_units(units.clone())),
                                None => self.prop_key(&x.key, &class_env)?,
                            };
                            let target = if x.is_static { &o } else { &p };
                            if matches!(x.kind, ast::MethodKind::Get | ast::MethodKind::Set) {
                                let mut property =
                                    target.borrow().own_prop(&k).cloned().unwrap_or(Property {
                                        value: JsValue::Undefined,
                                        writable: false,
                                        enumerable: false,
                                        configurable: true,
                                        getter: None,
                                        setter: None,
                                    });
                                if x.kind == ast::MethodKind::Get {
                                    property.getter = Some(method);
                                } else {
                                    property.setter = Some(method);
                                }
                                target.borrow_mut().define(&k, property).map_err(|_| {
                                    Throw::type_error("could not define class accessor")
                                })?;
                            } else {
                                target.borrow_mut().define(k, symbol_data(method, true, true)).unwrap();
                            }
                        }
                        ast::ClassMember::Method(_) => {}
                        ast::ClassMember::Field(field) => {
                            let key = if let ast::PropKey::Private(identifier) = &field.key {
                                FieldKey::Private(
                                    class_env
                                        .resolve_private(&identifier.name)
                                        .expect("collect_private_names recorded every private key"),
                                )
                            } else {
                                FieldKey::Public(self.prop_key(&field.key, &class_env)?)
                            };
                            let init = field.init.clone();
                            if field.is_static {
                                static_elements.push(StaticElement::Field { key, init });
                            } else {
                                instance_elements.push(InstanceElement::Field { key, init });
                            }
                        }
                        ast::ClassMember::StaticBlock(block) => {
                            static_elements.push(StaticElement::Block(block.clone()));
                        }
                    }
                }
                if !instance_elements.is_empty() {
                    o.borrow_mut().class_elements = Some(Rc::new(ClassElements {
                        instance: instance_elements,
                        class_env: class_env.clone(),
                        home: p.clone(),
                    }));
                }
                run_static_elements(self, &f, o.clone(), &class_env, &static_elements)?;
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
        let _ = document.define(
            "title",
            Property {
                value: JsValue::Undefined,
                writable: false,
                enumerable: true,
                configurable: true,
                getter: Some(title_getter),
                setter: Some(title_setter),
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
        let _ = element.define(
            "textContent",
            Property {
                value: JsValue::Undefined,
                writable: false,
                enumerable: true,
                configurable: true,
                getter: Some(getter),
                setter: Some(setter),
            },
        );
        let inner_getter = Self::native("get innerText", native_dom_text_get, &self.function_proto);
        let inner_setter = Self::native("set innerText", native_dom_text_set, &self.function_proto);
        let _ = element.define(
            "innerText",
            Property {
                value: JsValue::Undefined,
                writable: false,
                enumerable: true,
                configurable: true,
                getter: Some(inner_getter),
                setter: Some(inner_setter),
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
        let _ = element.define(
            "innerHTML",
            Property {
                value: JsValue::Undefined,
                writable: false,
                enumerable: true,
                configurable: true,
                getter: Some(inner_getter),
                setter: Some(inner_setter),
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
        let _ = element.define(
            "onclick",
            Property {
                value: JsValue::Null,
                writable: false,
                enumerable: true,
                configurable: true,
                getter: Some(onclick_getter),
                setter: Some(onclick_setter),
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

    fn array_length(&self, v: &JsValue) -> usize {
        if let JsValue::Object(o) = v { if let Some(BinaryState::View(view)) = binary_state(o) { if view.kind.is_some() { return binary_length(&view); } } }
        match v {
            JsValue::Object(o) => match o.borrow().get_own("length") {
                Some(JsValue::Num(n)) if n >= 0.0 => n as usize,
                _ => 0,
            },
            _ => 0,
        }
    }
    /// True when `v` is an array whose `[Symbol.iterator]` still resolves to
    /// the intrinsic `Array.prototype[Symbol.iterator]` (§22.1.3.34, aliased
    /// to `Array.prototype.values`), so the O(1) index-based fast path below
    /// is spec-equivalent to walking the general iterator protocol. A
    /// subclass override or a deleted `Array.prototype[Symbol.iterator]`
    /// must fall through to the slow path instead, so a missing/non-callable
    /// method still throws per GetIterator (§7.4.2).
    fn has_original_array_iterator(&mut self, v: &JsValue) -> Result<bool, Throw> {
        let method = self.get_prop(v, WellKnownSymbol::Iterator.symbol())?;
        let canonical = self.array_proto.borrow().get_own(WellKnownSymbol::Iterator.symbol());
        Ok(matches!(
            (&method, &canonical),
            (JsValue::Object(a), Some(JsValue::Object(b))) if Rc::ptr_eq(&a.0, &b.0)
        ))
    }
    fn iterate(&mut self, v: &JsValue) -> Result<Vec<JsValue>, Throw> {
        if let JsValue::Object(object) = v {
            if matches!(object.borrow().exotic, Exotic::Array) && self.has_original_array_iterator(v)? {
                let n = match object.borrow().get_own("length") {
                    Some(JsValue::Num(n)) => n as usize,
                    _ => 0,
                };
                return Ok((0..n)
                    .map(|n| self.get_prop(v, &n.to_string()))
                    .collect::<Result<_, _>>()?);
            }
            let iterator_method = self.get_prop(v, WellKnownSymbol::Iterator.symbol())?;
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
            // This eager collector is a temporary bridge until iterator
            // consumers advance lazily. Bound infinite iterators well below
            // the conformance runner's per-file deadline.
            for _ in 0..100_000usize {
                let result = match self.call_value(&next, v.clone(), Vec::new()) {
                    Ok(result) => result,
                    Err(error) => {
                        self.close_iterator(v);
                        return Err(error);
                    }
                };
                let result_value = match result {
                    JsValue::Object(_) => result,
                    _ => {
                        self.close_iterator(v);
                        return Err(Throw::type_error("iterator result is not an object"));
                    }
                };
                // IteratorComplete/IteratorValue (§7.4.6/§7.4.7) go through
                // the full [[Get]], not a raw own-property probe, so a
                // `done`/`value` accessor that throws is observed instead of
                // silently reading as absent (which would loop forever on an
                // iterator whose result objects define `value` as a getter).
                let done = match self.get_prop(&result_value, "done") {
                    Ok(value) => value.to_boolean(),
                    Err(error) => {
                        self.close_iterator(v);
                        return Err(error);
                    }
                };
                if done {
                    return Ok(values);
                }
                match self.get_prop(&result_value, "value") {
                    Ok(value) => values.push(value),
                    Err(error) => {
                        self.close_iterator(v);
                        return Err(error);
                    }
                }
            }
            self.close_iterator(v);
            return Err(Throw::range_error("iterator exceeded the iteration limit"));
        }
        match v {
            JsValue::Str(s) => {
                let mut values = Vec::with_capacity(s.len());
                let mut at = 0;
                while at < s.len() {
                    let first = s.units()[at];
                    if (0xd800..=0xdbff).contains(&first)
                        && s.units().get(at + 1).is_some_and(|next| (0xdc00..=0xdfff).contains(next))
                    {
                        values.push(JsValue::str_units(s.units()[at..at + 2].to_vec()));
                        at += 2;
                    } else {
                        values.push(JsValue::str_units(vec![first]));
                        at += 1;
                    }
                }
                Ok(values)
            }
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
            let result_value = match result {
                JsValue::Object(_) => result,
                _ => {
                    self.close_iterator(&iterator);
                    return Err(Throw::type_error("iterator result is not an object"));
                }
            };
            let done = match self.get_prop(&result_value, "done") {
                Ok(value) => value.to_boolean(),
                Err(error) => {
                    self.close_iterator(&iterator);
                    return Err(error);
                }
            };
            if done {
                break;
            }
            let value = match self.get_prop(&result_value, "value") {
                Ok(value) => value,
                Err(error) => {
                    self.close_iterator(&iterator);
                    return Err(error);
                }
            };
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
        if matches!(object.borrow().exotic, Exotic::Array) && self.has_original_array_iterator(value)? {
            return Ok(None);
        }
        let method = self.get_prop(value, WellKnownSymbol::Iterator.symbol())?;
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
            JsValue::BigInt(_) => "bigint",
            JsValue::Str(_) => "string",
            JsValue::Symbol(_) => "symbol",
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
        self.primitive_with_hint(v, false)
    }
    fn primitive_number_hint(&mut self, v: JsValue) -> Result<JsValue, Throw> {
        if !v.is_object() { return Ok(v); }
        let method = self.get_prop(&v, WellKnownSymbol::ToPrimitive.symbol())?;
        if !method.is_undefined() && !method.is_null() {
            if callable_function(&method).is_none() { return Err(Throw::type_error("@@toPrimitive is not callable")); }
            let result = self.call_value(&method, v.clone(), vec![JsValue::str("number")])?;
            if result.is_object() { return Err(Throw::type_error("@@toPrimitive returned an object")); }
            return Ok(result);
        }
        if let JsValue::Object(object) = &v {
            if object.borrow().get_own("valueOf").is_none() && object.borrow().get_own("toString").is_none() {
                for slot in ["[[BooleanData]]", "[[NumberData]]", "[[StringData]]"] {
                    if let Some(value) = object.borrow().get_own(slot) { return Ok(value); }
                }
            }
        }
        for name in ["valueOf", "toString"] {
            let method = self.get_prop(&v, name)?;
            if callable_function(&method).is_some() {
                let result = self.call_value(&method, v.clone(), Vec::new())?;
                if !result.is_object() { return Ok(result); }
            }
        }
        Err(Throw::type_error("cannot convert object to primitive"))
    }
    fn primitive_with_hint(&mut self, v: JsValue, string_hint: bool) -> Result<JsValue, Throw> {
        if !v.is_object() {
            return Ok(v);
        };
        let exotic = self.get_prop(&v, WellKnownSymbol::ToPrimitive.symbol())?;
        if !exotic.is_undefined() && !exotic.is_null() {
            if callable_function(&exotic).is_none() {
                return Err(Throw::type_error("@@toPrimitive is not callable"));
            }
            let hint = if string_hint { "string" } else { "default" };
            let result = self.call_value(&exotic, v.clone(), vec![JsValue::str(hint)])?;
            if result.is_object() { return Err(Throw::type_error("@@toPrimitive returned an object")); }
            return Ok(result);
        }
        if let JsValue::Object(object) = &v {
            for slot in ["[[BooleanData]]", "[[NumberData]]", "[[StringData]]"] {
                if let Some(value) = object.borrow().get_own(slot) {
                    return Ok(value);
                }
            }
        }
        let names = if string_hint { ["toString", "valueOf"] } else { ["valueOf", "toString"] };
        for n in names {
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
        Ok(self.to_js_string(v)?.to_utf8_lossy())
    }
    fn to_js_string(&mut self, v: &JsValue) -> Result<crate::value::JsString, Throw> {
        if v.is_object() {
            let p = self.primitive_with_hint(v.clone(), true)?;
            self.to_js_string(&p)
        } else {
            match v {
                JsValue::Str(s) => Ok(s.clone()),
                JsValue::Symbol(_) => Err(Throw::type_error("cannot convert Symbol to string")),
                _ => Ok(crate::value::JsString::from(crate::value::to_display_string(v))),
            }
        }
    }
    fn to_property_key(&mut self, v: &JsValue) -> Result<PropertyKey, Throw> {
        let primitive = self.primitive_with_hint(v.clone(), true)?;
        match primitive {
            JsValue::Symbol(symbol) => Ok(PropertyKey::Symbol(symbol)),
            other => Ok(PropertyKey::String(self.to_js_string(&other)?)),
        }
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
    fn instanceof(&mut self, a: &JsValue, b: &JsValue) -> Result<JsValue, Throw> {
        if !b.is_object() { return Err(Throw::type_error("right operand is not callable")); }
        let method = self.get_prop(b, WellKnownSymbol::HasInstance.symbol())?;
        if !method.is_null() && !method.is_undefined() {
            if callable_function(&method).is_none() {
                return Err(Throw::type_error("@@hasInstance is not callable"));
            }
            let result = self.call_value(&method, b.clone(), vec![a.clone()])?;
            return Ok(JsValue::Bool(result.to_boolean()));
        }
        if callable_function(b).is_none() {
            return Err(Throw::type_error("right operand is not callable"));
        }
        self.ordinary_has_instance(a, b)
    }
    fn ordinary_has_instance(&mut self, a: &JsValue, b: &JsValue) -> Result<JsValue, Throw> {
        let JsValue::Object(r) = b else { return Ok(JsValue::Bool(false)); };
        if r.borrow().func.is_none() { return Ok(JsValue::Bool(false)); }
        if !a.is_object() { return Ok(JsValue::Bool(false)); }
        let p = match self.get_prop(b, "prototype")? {
            JsValue::Object(p) => p,
            _ => return Err(Throw::type_error("prototype is not an object")),
        };
        if let (JsValue::Object(lhs), Some(fun)) = (a, r.borrow().func.clone()) {
            if let Some(JsValue::Str(lhs_name)) = lhs.borrow().get_own("name") {
                if !fun.name.is_empty() && lhs_name.units() == fun.name.encode_utf16().collect::<Vec<_>>() {
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
            // A Proxy (or WindowProxy) mid-chain must have its getPrototypeOf
            // trap observed, same as the Get(b, "prototype") call above.
            c = if let Some(result) = self.window_proxy_prototype(&o) {
                match result? { JsValue::Object(p) => Some(p), _ => None }
            } else if let Some(result) = self.proxy_get_prototype(&o) {
                match result? { JsValue::Object(p) => Some(p), _ => None }
            } else { o.borrow().proto.clone() };
        }
        Ok(JsValue::Bool(false))
    }


}

include!("interp_helpers.rs");
include!("interp_async.rs");
include!("interp_realms.rs");
include!("interp_eval_declarations.rs");
include!("interp_window_proxy.rs");
include!("interp_proxy.rs");
include!("interp_property_access.rs");
include!("interp_equality.rs");
include!("interp_install.rs");
include!("interp_symbol.rs");
include!("interp_builtins.rs");
include!("interp_array.rs");
include!("interp_array_protocol.rs");
include!("interp_array_factories.rs");
include!("interp_class_private.rs");
include!("interp_object_extra.rs");
include!("interp_errors.rs");
include!("interp_bound.rs");
include!("builtins_std_object_model.rs");
include!("builtins_std_reflect.rs");
include!("builtins_std_collections.rs");
include!("builtins_std_collection_iter.rs");
include!("builtins_std_iterator.rs");
include!("builtins_binary.rs");
include!("builtins_binary_bigint.rs");
include!("builtins_binary_base64.rs");
include!("builtins_intl_data.rs");
include!("builtins_intl_bcp47_data.rs");
include!("builtins_intl_unit_data.rs");
include!("builtins_intl_timezone.rs");
include!("builtins_intl.rs");
include!("builtins_intl_number.rs");
include!("builtins_intl_number_digits.rs");
include!("builtins_intl_plural.rs");
include!("builtins_intl_datetime.rs");
include!("builtins_intl_date_support.rs");
include!("builtins_intl_relative_list.rs");
include!("builtins_intl_duration.rs");
include!("builtins_intl_collator.rs");
include!("builtins_intl_segmenter.rs");
include!("builtins_intl_names.rs");
include!("builtins_intl_display.rs");
include!("builtins_intl_locale_methods.rs");
include!("builtins_temporal.rs");
include!("builtins_temporal_arithmetic.rs");
include!("builtins_temporal_epoch.rs");
include!("builtins_std_string.rs");
include!("builtins_std_number_math.rs");
include!("builtins_std_json_global.rs");
include!("structured_clone.rs");
include!("interp_regexp.rs");
include!("interp_tests.rs");

include!("interp_realm_tests.rs");
