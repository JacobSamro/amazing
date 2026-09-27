//! A small, panic-free tree-walking ECMAScript runtime.
//!
//! This is the Phase 6 execution milestone. It implements ordinary values,
//! objects, arrays, functions, closures, control flow, coercion and a small
//! set of ES built-ins using only `std`. Modules, generators and a
//! real asynchronous job queue remain explicit later milestones.
//!
//! Portions ported from lucid-softworks/lumen (crates/lumen/src/{eval,interpreter}.rs)
//! @ f6c9860d44fa52249cfd647122edea184d32c863, MIT: explicit resource management's `using_stack`
//! disposal-frame stack, the `Disposable` record, and `add_disposable`/`dispose_method`/
//! `dispose_frame` (see their doc comments for the exact adaptation).

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::ast::{self, Binding, ExprKind, ForInit, ForLeft, MemberProp, PropKey, StmtKind};
use crate::env::{EnvHandle, EnvKind};
use crate::value::{
    number_to_string, ClassElements, DerivedConstructor, DisposableStackEntry, DisposableStackState,
    Exotic, FieldKey, FunctionBody, HostCallPolicy,
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

#[path = "interp_event_handler.rs"]
mod event_handler;

#[path = "interp_bindings.rs"]
mod bindings;
pub use bindings::BindingPropertyFlags;
#[path = "interp_legacy.rs"]
mod legacy;
#[path = "interp_async_iter.rs"]
mod async_iter;
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
    /// %GeneratorPrototype%: the shared object every generator function's
    /// own (per-function) `.prototype` object inherits from -- distinct
    /// from `generator_function_proto` (%GeneratorFunction.prototype%,
    /// the FUNCTION's own `[[Prototype]]`). `generator_function_proto`'s
    /// own "prototype" property points at this object.
    pub generator_proto: Obj,
    /// %AsyncGeneratorPrototype%, the async equivalent of `generator_proto`.
    pub async_generator_proto: Obj,
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
    /// Explicit resource management (`using`/`await using`): a stack of disposal frames, one per
    /// block/function-body/for-head/for-of-iteration that actually declares a resource. Resources
    /// are disposed in reverse declaration order when that scope exits (see `dispose_frame`).
    /// Ported from lucid-softworks/lumen (crates/lumen/src/{eval,interpreter}.rs) @ f6c9860d, MIT.
    using_stack: Vec<Vec<Disposable>>,
    error_brands: std::collections::HashSet<u64>,
    legacy_iterator_proto: Obj,
    pair_hooks: HashMap<u64, PairIterableHooks>,
    pair_iterator_proto: Obj,
    regexp_string_iterator_proto: Obj,
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
        // %GeneratorPrototype%'s own [[Prototype]] is %IteratorPrototype%
        // (installed later by install_std_iterator_helpers and re-parented
        // below once it exists); %AsyncGeneratorPrototype% keeps a plain
        // Object.prototype parent since this crate has no %AsyncIterator-
        // Prototype% singleton yet (no test in this ticket's scope needs it).
        let generator_proto = Obj(Rc::new(RefCell::new({
            let mut o = JsObject::ordinary();
            o.proto = Some(object_proto.clone());
            o
        })));
        let async_generator_proto = Obj(Rc::new(RefCell::new({
            let mut o = JsObject::ordinary();
            o.proto = Some(object_proto.clone());
            o
        })));
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
        let regexp_string_iterator_proto = Obj(Rc::new(RefCell::new({
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
            generator_proto,
            async_generator_proto,
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
            using_stack: Vec::new(),
            error_brands: std::collections::HashSet::new(),
            legacy_iterator_proto,
            pair_hooks: HashMap::new(),
            pair_iterator_proto,
            regexp_string_iterator_proto,
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
        self.hoist_inner(body, env, false, &mut Vec::new())
    }

    /// `nested` is `false` only for the direct statement list of a function/script/module/eval
    /// body: a `FunctionDeclaration` found there binds eagerly and unconditionally, as usual.
    /// Recursing into a further block/if/loop/labeled body sets `nested = true`, where a *plain*
    /// (non-async, non-generator) function declaration is instead an Annex B.3.3 candidate: in
    /// sloppy mode, and unless shadowed by an intervening `let`/`const`/class/`using` declaration
    /// of the same name (tracked via `blocked`), it gets an undefined-initialized legacy `var`
    /// binding in the enclosing scope (`hoist_var`), synced with the block's own lexical binding
    /// when the `FunctionDeclaration` statement is actually evaluated -- see `StmtKind::Function`
    /// and `StmtKind::Block`'s block-entry hoist in interp_stmt.rs. `StmtKind::Try` deliberately
    /// starts a *fresh* top-level hoist (unchanged from before this split): Annex B promotion out
    /// of try/catch/finally bodies is a known, documented gap.
    /// Adapted from lucid-softworks/lumen (crates/lumen/src/interpreter.rs, collect_annexb_funcs)
    /// @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
    fn hoist_inner(&mut self, body: &[ast::Stmt], env: &EnvHandle, nested: bool, blocked: &mut Vec<String>) -> Result<(), Throw> {
        for s in body {
            match &s.kind {
                StmtKind::Var(d) if d.kind == ast::VarKind::Var => {
                    for x in &d.decls {
                        self.hoist_binding(&x.binding, env)?;
                    }
                }
                StmtKind::Function(f) => {
                    if let Some(n) = &f.name {
                        if !nested {
                            let v = self.make_function(f, env);
                            if self.in_eval { self.create_eval_binding(env, &n.name, Some(v))?; }
                            else { env.declare_var(&n.name, v)?; }
                        } else if !self.strict_eval_context && !f.is_async && !f.is_generator {
                            // Record THIS declaration's own individual eligibility, keyed by its
                            // own AST node address, not just "does the shared var exist" -- the
                            // sync step in interp_stmt.rs's StmtKind::Function execution arm reads
                            // this to decide whether *it specifically* may promote, since several
                            // same-named FunctionDeclarations at different nesting depths can have
                            // different `blocked` outcomes even though they'd all otherwise see the
                            // same (or no) outer var cell. Never cleared: re-derived fresh, and
                            // correctly, on every hoist pass this exact static node is visited,
                            // including from a nested/recursive call's own hoist() in between.
                            let key = f.as_ref() as *const ast::FunctionItem as usize;
                            if blocked.iter().any(|b| b == &n.name) {
                                annex_b_eligible_set(|set| { set.remove(&key); });
                            } else {
                                annex_b_eligible_set(|set| { set.insert(key); });
                                env.hoist_var(&n.name)?;
                            }
                        }
                    }
                }
                StmtKind::Block(b) => {
                    let added = block_lexical_names(&b.body);
                    let pushed = push_blocked(blocked, added);
                    self.hoist_inner(&b.body, env, true, blocked)?;
                    blocked.truncate(blocked.len() - pushed);
                }
                StmtKind::If { then, els, .. } => {
                    self.hoist_inner(std::slice::from_ref(then), env, true, blocked)?;
                    if let Some(e) = els {
                        self.hoist_inner(std::slice::from_ref(e), env, true, blocked)?;
                    }
                }
                StmtKind::While { body, .. } | StmtKind::DoWhile { body, .. } => {
                    self.hoist_inner(std::slice::from_ref(body), env, true, blocked)?
                }
                StmtKind::For { init, body, .. } => {
                    if let Some(ForInit::Decl(d)) = init {
                        self.hoist_var(d, env)?;
                    }
                    let added = match init {
                        Some(ForInit::Decl(d)) if d.kind != ast::VarKind::Var =>
                            d.decls.iter().flat_map(|x| binding_names(&x.binding)).collect(),
                        _ => Vec::new(),
                    };
                    let pushed = push_blocked(blocked, added);
                    self.hoist_inner(std::slice::from_ref(body), env, true, blocked)?;
                    blocked.truncate(blocked.len() - pushed);
                }
                StmtKind::ForIn { left, body, .. } | StmtKind::ForOf { left, body, .. }
                | StmtKind::ForAwaitOf { left, body, .. } => {
                    if let ForLeft::Decl(d) = left {
                        self.hoist_var(d, env)?;
                    }
                    let added = match left {
                        ForLeft::Decl(d) if d.kind != ast::VarKind::Var =>
                            d.decls.iter().flat_map(|x| binding_names(&x.binding)).collect(),
                        _ => Vec::new(),
                    };
                    let pushed = push_blocked(blocked, added);
                    self.hoist_inner(std::slice::from_ref(body), env, true, blocked)?;
                    blocked.truncate(blocked.len() - pushed);
                }
                StmtKind::Labeled { body, .. } => self.hoist_inner(std::slice::from_ref(body), env, true, blocked)?,
                StmtKind::With { body, .. } => self.hoist_inner(std::slice::from_ref(body), env, true, blocked)?,
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
include!("interp_disposable.rs");
include!("interp_coroutine.rs");
include!("interp_stmt.rs");
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
