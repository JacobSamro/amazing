//! Web Storage, IndexedDB, and Cache API driver bindings.
use super::*;
use std::{collections::{BTreeMap,VecDeque}, path::PathBuf, rc::Weak, sync::{Arc,Mutex,Weak as SyncWeak}};
use amazing_bindings::storage_interfaces;
use amazing_storage::{core::{BucketKey, DiskStore, StorageKey, QuotaLedger}, web::{PersistentWebStorage, StorageChange}, idb::{self, IndexedDb, StructuredValue, Key, KeyRange, Direction, Mode, Durability, Transaction as IdbTransaction, Cursor as IdbCursor}, cache::{self, PersistentCacheStorage}, manager::StorageManager, buckets::{BucketRegistry, Policy, valid_bucket_name}, locks::{LockCoordinator, Mode as LockMode, Options as LockOptions}};
use std::sync::atomic::{AtomicU64, Ordering};
use amazing_js::{SerializedHostValue, StructuredCloneHost};

struct StorageCloneHost { state: Rc<RefCell<BindingState>> }
impl StorageCloneHost {
    fn field(out: &mut Vec<u8>, value: &[u8]) { out.extend_from_slice(&(value.len() as u32).to_le_bytes()); out.extend_from_slice(value); }
    fn read_field<'a>(bytes: &mut &'a [u8]) -> Result<&'a [u8], Throw> {
        if bytes.len() < 4 { return Err(Throw::error("DataCloneError", "invalid stored host value")); }
        let len = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
        *bytes = &bytes[4..];
        if bytes.len() < len { return Err(Throw::error("DataCloneError", "invalid stored host value")); }
        let (value, tail) = bytes.split_at(len);
        *bytes = tail;
        Ok(value)
    }
}
impl StructuredCloneHost for StorageCloneHost {
    fn serialize_host(&mut self, value: &JsValue) -> Result<Option<SerializedHostValue>, Throw> {
        let JsValue::Object(object) = value else { return Ok(None); };
        let pointer = Rc::as_ptr(&object.0) as usize;
        let state = self.state.borrow();
        let Some(slot) = state.web_object_handles.get(&pointer) else { return Ok(None); };
        let (type_name, blob, file) = match &state.web_objects[*slot] {
            WebObject::Blob(blob) => ("Blob", blob, None),
            WebObject::File(file) => ("File", &file.blob, Some(file)),
            _ => return Ok(None),
        };
        let mut bytes = Vec::new();
        Self::field(&mut bytes, &blob.bytes);
        Self::field(&mut bytes, blob.content_type.as_bytes());
        if let Some(file) = file {
            Self::field(&mut bytes, file.name.as_bytes());
            bytes.extend_from_slice(&file.last_modified.to_le_bytes());
        }
        Ok(Some(SerializedHostValue { type_name: type_name.into(), bytes }))
    }
    fn deserialize_host(&mut self, value: &SerializedHostValue, target: &mut Interpreter) -> Result<JsValue, Throw> {
        let mut bytes = value.bytes.as_slice();
        let data = Self::read_field(&mut bytes)?.to_vec();
        let content_type = String::from_utf8(Self::read_field(&mut bytes)?.to_vec())
            .map_err(|_| Throw::error("DataCloneError", "invalid Blob type"))?;
        let blob = Blob::from_bytes(data, &content_type);
        let mut adapter = RuntimeBindingHost { js: target, state: self.state.clone() };
        match value.type_name.as_str() {
            "Blob" if bytes.is_empty() => adapter.blob_new(blob),
            "File" => {
                let name = String::from_utf8(Self::read_field(&mut bytes)?.to_vec())
                    .map_err(|_| Throw::error("DataCloneError", "invalid File name"))?;
                if bytes.len() != 8 { return Err(Throw::error("DataCloneError", "invalid File timestamp")); }
                let last_modified = i64::from_le_bytes((*bytes).try_into().unwrap());
                adapter.file_new(File { blob, name, last_modified })
            }
            _ => Err(Throw::error("DataCloneError", "unknown stored host value")),
        }
    }
}

struct TestStorageRoot { path: PathBuf }

impl TestStorageRoot {
    fn new() -> Rc<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let sequence=NEXT.fetch_add(1,Ordering::Relaxed);
        let nonce=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let path=std::env::temp_dir().join(format!("amazing-storage-test-{}-{nonce}-{sequence}",std::process::id()));
        Rc::new(Self{path})
    }
}

impl Drop for TestStorageRoot {
    fn drop(&mut self) {let _=std::fs::remove_dir_all(&self.path);}
}

#[cfg(test)]
thread_local! {
    static TEST_STORAGE_OVERRIDE: RefCell<Option<Rc<TestStorageRoot>>>=const {RefCell::new(None)};
}

pub(super) struct StorageRuntime {
    web: Rc<RefCell<PersistentWebStorage>>,
    idb: Rc<RefCell<IndexedDb>>,
    cache: Rc<RefCell<PersistentCacheStorage>>,
    manager: Rc<RefCell<StorageManager>>,
    buckets: Rc<RefCell<BucketRegistry>>,
    locks: Rc<RefCell<LockCoordinator>>,
    lock_client_id: u64,
    lock_callbacks: HashMap<u64,LockCallback>,
    objects: Vec<StorageObject>,
    wrappers: HashMap<usize, JsValue>,
    pending: VecDeque<StorageJob>,
    blocked_opens: VecDeque<(BucketKey,String,usize)>,
    blocked_deletes: VecDeque<(BucketKey,String,usize)>,
    tick_scheduled: bool,
    _test_root: Option<Rc<TestStorageRoot>>,
}

thread_local! {
    static SHARED_WEB_STORAGE: RefCell<HashMap<PathBuf,Weak<RefCell<PersistentWebStorage>>>>=RefCell::new(HashMap::new());
    static SHARED_INDEXED_DB: RefCell<HashMap<PathBuf,Weak<RefCell<IndexedDb>>>>=RefCell::new(HashMap::new());
    static SHARED_CACHE: RefCell<HashMap<PathBuf,Weak<RefCell<PersistentCacheStorage>>>>=RefCell::new(HashMap::new());
    static SHARED_MANAGER: RefCell<HashMap<PathBuf,Weak<RefCell<StorageManager>>>>=RefCell::new(HashMap::new());
    static SHARED_BUCKETS: RefCell<HashMap<PathBuf,Weak<RefCell<BucketRegistry>>>>=RefCell::new(HashMap::new());
    static SHARED_QUOTA: RefCell<HashMap<PathBuf,SyncWeak<Mutex<QuotaLedger>>>>=RefCell::new(HashMap::new());
    static SHARED_LOCKS: RefCell<HashMap<PathBuf,Weak<RefCell<LockCoordinator>>>>=RefCell::new(HashMap::new());
    static STORAGE_DOCUMENTS: RefCell<HashMap<PathBuf,Vec<Weak<RefCell<BindingState>>>>>=RefCell::new(HashMap::new());
}

#[derive(Clone)]
pub(super) struct StorageEventDelivery {
    change: StorageChange,
    url: String,
}

enum StorageObject {
    Area { session: bool },
    Factory { bucket: String },
    Request { result: JsValue, error: JsValue, ready: bool, source: JsValue, transaction: JsValue, handlers: HashMap<String, JsValue>, listeners: Vec<(String, JsValue)> },
    // `key` is cached at connection-open time (STORE-6 regression fix): deriving it by
    // looking up the live `idb::Connection` on every member access throws once the
    // connection is closed, even for read-only getters (name/objectStoreNames) and for
    // the idempotent second `close()` call that the spec and WPT cleanup helpers rely
    // on. See the regression test below for the hang this caused.
    Database { connection: u64, name: String, key: BucketKey, handlers: HashMap<String, JsValue>, listeners: Vec<(String, JsValue)> },
    Transaction { connection: u64, id: u64, value: Option<IdbTransaction>, mode: Mode, scope: Vec<String>, queued: bool, active_for_requests: bool, handlers: HashMap<String, JsValue>, listeners: Vec<(String, JsValue)> },
    Store { transaction: usize, name: String, key_path_value: Option<JsValue> },
    Index { transaction: usize, store: String, name: String, key_path_value: Option<JsValue>, object_store_value: Option<JsValue> },
    Cursor { cursor: IdbCursor, transaction: usize, source: usize, request: usize, with_value: bool },
    Range(KeyRange),
    // getAllRecords()'s IDBRecord result: a plain readonly-attribute holder, but WPT's
    // assert_idl_attribute requires key/primaryKey/value to be inherited getters on the
    // interface prototype, not own data properties -- so this goes through the same
    // storage_allocate/storage_slot handle machinery as every other IDB wrapper instead
    // of a bare object with own properties.
    Record { key: JsValue, primary_key: JsValue, value: JsValue },
    CacheStorage { bucket: String },
    Cache { name: String },
    Manager,
    LockManager,
    Lock { name: String, mode: amazing_storage::locks::Mode },
    BucketManager,
    Bucket { name: String, epoch: u64 },
}

enum StorageJob {
    LockGranted { id: u64 },
    RequestSuccess { request: usize, result: JsValue },
    RequestError { request: usize, error: JsValue },
    DeleteSuccess { request: usize, old: u64 },
    OpenUpgrade { request: usize, connection: u64, name: String, key: BucketKey, old: u64, new: u64 },
    OpenBlocked { request: usize, old: u64, new: Option<u64> },
    DatabaseVersionChange { connection: u64, old: u64, new: Option<u64> },
    DispatchListener { slot: usize, event: JsValue, callbacks: VecDeque<JsValue> },
    TransactionComplete { transaction: usize },
    StorageEvent(StorageEventDelivery),
}

struct LockCallback {
    callback: JsValue,
    promise: JsValue,
    key: StorageKey,
    name: String,
    mode: LockMode,
    started: bool,
    acquired: bool,
}

impl Drop for StorageRuntime {
    fn drop(&mut self) {self.locks.borrow_mut().release_client(self.lock_client_id);}
}

enum StoreResult {
    None,
    Count(usize),
    Key(Key),
    OptionalKey(Option<Key>),
    Value(Option<StructuredValue>),
}

impl RuntimeBindings {
    pub(crate) fn schedule_storage_events(&self, js: &mut Interpreter) -> Result<(), Throw> {
        let mut adapter = RuntimeBindingHost { js, state: self.state.clone() };
        loop {
            let delivery = adapter.state.borrow_mut().storage_inbox.pop_front();
            let Some(delivery) = delivery else { break };
            if let Err(error) = adapter.storage_schedule(StorageJob::StorageEvent(delivery.clone())) {
                adapter.state.borrow_mut().storage_inbox.push_front(delivery);
                return Err(error);
            }
        }
        let grants={let state=adapter.state.borrow();let Some(storage)=state.storage.as_ref() else{return Ok(())};
            let locks=storage.locks.borrow();
            storage.lock_callbacks.iter().filter_map(|(id,callback)|{
                if callback.started{return None}
                locks.query(&callback.key).held.iter().any(|lock|lock.id==*id).then_some(*id)
            }).collect::<Vec<_>>()};
        for id in grants {
            if let Some(callback)=adapter.state.borrow_mut().storage.as_mut().unwrap().lock_callbacks.get_mut(&id){
                callback.started=true;callback.acquired=true;
            }
            adapter.storage_schedule(StorageJob::LockGranted{id})?;
        }
        Ok(())
    }

    /// IndexedDB transactions stop accepting new requests after the task's
    /// microtask checkpoint. Request event dispatch reactivates its transaction.
    pub(crate) fn storage_deactivate_transactions(&self) {
        let mut state=self.state.borrow_mut();
        if let Some(storage)=state.storage.as_mut(){
            for object in &mut storage.objects {
                if let StorageObject::Transaction{active_for_requests,..}=object {*active_for_requests=false;}
            }
        }
    }
}

impl RuntimeBindingHost<'_> {
    fn dispatch_storage_event(&mut self, delivery: StorageEventDelivery) -> Result<(), Throw> {
        let window = self.js.binding_global_this();
        let event = invoke(self, Operation::EventConstructor, JsValue::Undefined,
            &[JsValue::str("storage")])?;
        let units = |value: Option<Vec<u16>>| value.map_or(JsValue::Null,
            |units| JsValue::str(JsString::from_units(units)));
        for (name, value) in [
            ("key", units(delivery.change.key)),
            ("oldValue", units(delivery.change.old_value)),
            ("newValue", units(delivery.change.new_value)),
            ("url", JsValue::str(delivery.url)),
            ("storageArea", self.js.binding_get_property(&window, "localStorage")?),
            ("target", window.clone()),
            ("currentTarget", window.clone()),
        ] {
            self.js.binding_define_data_property(&event, name, value, BindingPropertyFlags::METHOD)?;
        }
        // Window listeners share the DOM EventTarget registry, including
        // capture, once, and removal semantics. The property handler is kept
        // below until onstorage joins the common event-handler path.
        invoke(self, Operation::DispatchEvent, window.clone(), &[event.clone()])?;
        let handler = self.js.binding_get_property(&window, "onstorage")?;
        if self.js.binding_is_callable(&handler) {
            let _ = self.js.binding_call(&handler, window, vec![event]);
        }
        Ok(())
    }

    pub(super) fn storage_lexical_areas(&mut self) -> Result<(JsValue, JsValue), Throw> {
        let local=self.storage_allocate(SI::Storage,StorageObject::Area{session:false})?;
        let session=self.storage_allocate(SI::Storage,StorageObject::Area{session:true})?;
        Ok((local,session))
    }
}

const STORAGE_BRAND_BASE: u64 = 100;
const STORAGE_QUOTA: u64 = 50 * 1024 * 1024;

impl RuntimeBindingHost<'_> {
    pub(super) fn install_storage_interfaces(&mut self) -> Result<(), Throw> {
        for definition in storage_interfaces::INTERFACES {
            let parent = definition.parent
                .and_then(|kind| self.state.borrow().storage_classes.get(&kind).map(|pair|pair.0.clone()))
                .unwrap_or_else(||self.js.binding_object_prototype());
            let prototype=self.js.binding_create_object(Some(&parent))?;
            self.js.binding_define_symbol_data_property(&prototype,
                amazing_js::value::WellKnownSymbol::ToStringTag.symbol(),
                JsValue::str(definition.name),
                BindingPropertyFlags{writable:false,enumerable:false,configurable:true})?;
            let entry=if definition.kind==SI::IDBVersionChangeEvent{
                NativeEntry::Storage(SI::IDBVersionChangeEvent,"constructor",SM::StaticMethod)
            }else{NativeEntry::IllegalConstructor};
            let constructor=self.native(definition.name,if definition.kind==SI::IDBVersionChangeEvent{1}else{0},entry,HostCallPolicy::ConstructOnly);
            self.define_data_property(&constructor,"prototype",prototype.clone(),false,false,false)?;
            self.define_data_property(&prototype,"constructor",constructor.clone(),true,false,true)?;
            let mut getters:HashMap<&'static str,JsValue>=HashMap::new();
            for member in definition.members {
                let function=self.native(member.name,member.length,
                    NativeEntry::Storage(definition.kind,member.name,member.kind),HostCallPolicy::CallOnly);
                match member.kind {
                    SM::Method=>self.define_data_property(&prototype,member.name,function,true,false,true)?,
                    SM::StaticMethod=>self.define_data_property(&constructor,member.name,function,true,false,true)?,
                    SM::Getter=>{getters.insert(member.name,function.clone());
                        self.define_accessor_property(&prototype,member.name,Some(function),None,true,true)?;},
                    SM::Setter=>self.define_accessor_property(&prototype,member.name,getters.get(member.name).cloned(),Some(function),true,true)?,
                }
            }
            self.state.borrow_mut().storage_classes.insert(definition.kind,(prototype,constructor.clone()));
            let global=self.js.binding_global_object();
            self.define_data_property(&global,definition.name,constructor,true,false,true)?;
        }
        let global=self.js.binding_global_object();
        for name in ["localStorage","sessionStorage"] {
            let getter=self.native(name,0,NativeEntry::Storage(SI::Storage,name,SM::Getter),HostCallPolicy::CallOnly);
            self.define_accessor_property(&global,name,Some(getter),None,true,true)?;
        }
        // IA-REAL-2: `indexedDB`/`caches`/`navigator.storage`/`.locks`/`.storageBuckets` used
        // to be allocated eagerly here, and `storage_allocate` unconditionally opens the
        // on-disk store as its first step. That made every single page load pay disk I/O
        // for storage before any script ran, and made a stale/corrupt on-disk store fail
        // page load entirely rather than just the storage APIs a page happens to touch.
        // These five singletons are installed as getters instead (mirroring the existing
        // localStorage/sessionStorage lazy pattern below) so the store opens lazily, on
        // first actual use.
        for (name,interface) in [
            ("indexedDB",SI::IDBFactory),
            ("caches",SI::CacheStorage),
        ] {
            let getter=self.native(name,0,NativeEntry::Storage(interface,"!singleton",SM::Getter),HostCallPolicy::CallOnly);
            self.define_accessor_property(&global,name,Some(getter),None,true,true)?;
        }
        let navigator=self.js.binding_get_property(&global,"navigator")?;
        let navigator=if matches!(navigator,JsValue::Object(_)){navigator}else{
            let value=self.js.binding_create_object(None)?;
            self.define_data_property(&global,"navigator",value.clone(),true,false,true)?;
            value
        };
        for (name,interface) in [
            ("storage",SI::StorageManager),
            ("locks",SI::LockManager),
            ("storageBuckets",SI::StorageBucketManager),
        ] {
            let getter=self.native(name,0,NativeEntry::Storage(interface,"!singleton",SM::Getter),HostCallPolicy::CallOnly);
            self.define_accessor_property(&navigator,name,Some(getter),None,true,true)?;
        }
        self.install_storage_legacy();
        Ok(())
    }
    /// Lazily open storage and allocate (or reuse) the one wrapper object for a
    /// per-realm storage singleton (IDBFactory/CacheStorage/StorageManager/
    /// LockManager/StorageBucketManager), on first access rather than at
    /// realm/document init (IA-REAL-2).
    fn storage_singleton(&mut self,interface:SI)->Result<JsValue,Throw>{
        self.storage_ensure()?;
        let object=match interface {
            SI::IDBFactory=>StorageObject::Factory{bucket:"default".into()},
            SI::CacheStorage=>StorageObject::CacheStorage{bucket:"default".into()},
            SI::StorageManager=>StorageObject::Manager,
            SI::LockManager=>StorageObject::LockManager,
            SI::StorageBucketManager=>StorageObject::BucketManager,
            _=>return Err(Throw::type_error("Not a storage singleton")),
        };
        let existing={let state=self.state.borrow();let storage=state.storage.as_ref().unwrap();
            storage.objects.iter().position(|o|std::mem::discriminant(o)==std::mem::discriminant(&object))};
        if let Some(slot)=existing{return self.storage_wrapper(slot)}
        self.storage_allocate(interface,object)
    }
    fn storage_profile_path() -> std::path::PathBuf {
        if let Some(path)=std::env::var_os("AMAZING_STORAGE_DIR") {return path.into()}
        if let Some(path)=std::env::var_os("LOCALAPPDATA") {return std::path::PathBuf::from(path).join("Amazing").join("Storage")}
        if let Some(path)=std::env::var_os("XDG_DATA_HOME") {return std::path::PathBuf::from(path).join("amazing").join("storage")}
        std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir)
            .join(".local").join("share").join("amazing").join("storage")
    }
    fn storage_ensure(&mut self) -> Result<(), Throw> {
        if self.state.borrow().storage.is_some(){return Ok(())}
        let isolated=std::env::var_os("AMAZING_STORAGE_DIR").is_none()
            &&(cfg!(test)||std::env::var_os("NEXTEST").as_deref()==Some(std::ffi::OsStr::new("1")));
        let test_root=if isolated {
            #[cfg(test)]
            {Some(TEST_STORAGE_OVERRIDE.with(|slot|slot.borrow().clone()).unwrap_or_else(TestStorageRoot::new))}
            #[cfg(not(test))]
            {Some(TestStorageRoot::new())}
        }else{None};
        let path=test_root.as_ref().map(|root|root.path.clone()).unwrap_or_else(Self::storage_profile_path);
        let quota=SHARED_QUOTA.with(|shared|{
            if let Some(value)=shared.borrow().get(&path).and_then(SyncWeak::upgrade){return value}
            let value=Arc::new(Mutex::new(QuotaLedger::new(STORAGE_QUOTA)));
            shared.borrow_mut().insert(path.clone(),Arc::downgrade(&value));value
        });
        let web=SHARED_WEB_STORAGE.with(|shared|->Result<_,Throw>{
            if let Some(value)=shared.borrow().get(&path).and_then(Weak::upgrade){return Ok(value)}
            let mut disk=DiskStore::open(path.join("web"),STORAGE_QUOTA)
                .map_err(|e|Throw::type_error(format!("storage open: {e:?}")))?;
            disk.attach_quota(quota.clone());
            let value=Rc::new(RefCell::new(PersistentWebStorage::open(disk,10*1024*1024)
                .map_err(|e|Throw::type_error(format!("Web Storage open: {e:?}")))?));
            shared.borrow_mut().insert(path.clone(),Rc::downgrade(&value));Ok(value)
        })?;
        let idb=SHARED_INDEXED_DB.with(|shared|->Result<_,Throw>{
            if let Some(value)=shared.borrow().get(&path).and_then(Weak::upgrade){return Ok(value)}
            let mut disk=DiskStore::open(path.join("idb"),STORAGE_QUOTA)
                .map_err(|e|Throw::type_error(format!("IndexedDB open: {e:?}")))?;
            disk.attach_quota(quota.clone());
            let value=Rc::new(RefCell::new(IndexedDb::with_disk(disk)
                .map_err(|e|Throw::type_error(format!("IndexedDB decode: {e:?}")))?));
            shared.borrow_mut().insert(path.clone(),Rc::downgrade(&value));Ok(value)
        })?;
        let cache=SHARED_CACHE.with(|shared|->Result<_,Throw>{
            if let Some(value)=shared.borrow().get(&path).and_then(Weak::upgrade){return Ok(value)}
            let mut disk=DiskStore::open(path.join("cache"),STORAGE_QUOTA)
                .map_err(|e|Throw::type_error(format!("Cache open: {e:?}")))?;
            disk.attach_quota(quota.clone());
            let value=Rc::new(RefCell::new(PersistentCacheStorage::open(disk)
                .map_err(|e|Throw::type_error(format!("Cache decode: {e:?}")))?));
            shared.borrow_mut().insert(path.clone(),Rc::downgrade(&value));Ok(value)
        })?;
        let manager=SHARED_MANAGER.with(|shared|->Result<_,Throw>{
            if let Some(value)=shared.borrow().get(&path).and_then(Weak::upgrade){return Ok(value)}
            let disk=DiskStore::open(path.join("manager"),STORAGE_QUOTA)
                .map_err(|e|Throw::type_error(format!("StorageManager open: {e:?}")))?;
            let value=Rc::new(RefCell::new(StorageManager::open(disk,quota)
                .map_err(|e|Throw::type_error(format!("StorageManager decode: {e:?}")))?));
            shared.borrow_mut().insert(path.clone(),Rc::downgrade(&value));Ok(value)
        })?;
        let locks=SHARED_LOCKS.with(|shared|{
            if let Some(value)=shared.borrow().get(&path).and_then(Weak::upgrade){return value}
            let value=Rc::new(RefCell::new(LockCoordinator::default()));
            shared.borrow_mut().insert(path.clone(),Rc::downgrade(&value));value
        });
        let buckets=SHARED_BUCKETS.with(|shared|->Result<_,Throw>{
            if let Some(value)=shared.borrow().get(&path).and_then(Weak::upgrade){return Ok(value)}
            let disk=DiskStore::open(path.join("buckets"),STORAGE_QUOTA)
                .map_err(|e|Throw::type_error(format!("Storage Buckets open: {e:?}")))?;
            let value=Rc::new(RefCell::new(BucketRegistry::load(disk)
                .map_err(|e|Throw::type_error(format!("Storage Buckets decode: {e:?}")))?));
            shared.borrow_mut().insert(path.clone(),Rc::downgrade(&value));Ok(value)
        })?;
        let lock_client_id=self.state.borrow().arena;
        self.state.borrow_mut().storage=Some(StorageRuntime {web,idb,cache,manager,buckets,locks,lock_client_id,lock_callbacks:HashMap::new(),objects:Vec::new(),wrappers:HashMap::new(),pending:VecDeque::new(),blocked_opens:VecDeque::new(),blocked_deletes:VecDeque::new(),tick_scheduled:false,_test_root:test_root});
        STORAGE_DOCUMENTS.with(|documents| {
            let mut documents = documents.borrow_mut();
            let peers = documents.entry(path).or_default();
            peers.retain(|peer| peer.upgrade().is_some());
            peers.push(Rc::downgrade(&self.state));
        });
        Ok(())
    }
    fn storage_key(&self) -> Result<StorageKey, Throw> {
        use amazing_fetch::{HttpUrl,Site,psl::PublicSuffixList};
        let url=self.state.borrow().document_url.clone().ok_or_else(||Throw::type_error("Storage requires a document URL"))?;
        let parsed=HttpUrl::parse(&url).map_err(|_|Throw::type_error("Storage requires an HTTP(S) origin"))?;
        let origin=parsed.origin();
        let site=match origin.site(&PublicSuffixList) {
            Site::Tuple {scheme,domain}=>format!("{scheme}://{domain}"),
            Site::Opaque(_)=>return Err(Throw::type_error("Storage is unavailable for opaque origins")),
        };
        Ok(StorageKey::new(origin.serialize(),site))
    }
    fn storage_bucket(&self) -> Result<BucketKey, Throw> {Ok(BucketKey::new(self.storage_key()?,"default"))}
    fn storage_allocate(&mut self,kind:SI,object:StorageObject) -> Result<JsValue, Throw> {
        self.storage_ensure()?;
        let (slot,prototype)={
            let mut state=self.state.borrow_mut();
            let storage=state.storage.as_mut().expect("storage initialized");
            let slot=storage.objects.len();storage.objects.push(object);
            let prototype=state.storage_classes.get(&kind).ok_or_else(||Throw::type_error("Storage class not installed"))?.0.clone();
            (slot,prototype)
        };
        let arena=self.state.borrow().arena;
        let value=self.js.binding_wrap_host(HostHandle {arena,slot:slot as u64,generation:1,brand:STORAGE_BRAND_BASE+kind as u64},&prototype)?;
        self.state.borrow_mut().storage.as_mut().expect("storage initialized").wrappers.insert(slot,value.clone());
        // IDBRecord's key/primaryKey/value must stay inherited (WPT's assert_idl_attribute
        // requires them NOT to be own properties); every other storage wrapper still needs
        // materialize_accessors for its host-object property lookup to see prototype getters.
        if kind!=SI::IDBRecord {self.materialize_accessors(&value);}
        Ok(value)
    }
    fn storage_slot(&self,value:&JsValue,kind:SI)->Result<usize,Throw>{
        let handle=self.js.binding_host_handle(value,STORAGE_BRAND_BASE+kind as u64)
            .or_else(||(kind==SI::IDBRequest).then(||self.js.binding_host_handle(value,STORAGE_BRAND_BASE+SI::IDBOpenDBRequest as u64)).flatten())
            .or_else(||(kind==SI::IDBCursor).then(||self.js.binding_host_handle(value,STORAGE_BRAND_BASE+SI::IDBCursorWithValue as u64)).flatten())
            .ok_or_else(||Throw::type_error("Incompatible storage receiver"))?;
        if handle.arena!=self.state.borrow().arena||handle.generation!=1{return Err(Throw::type_error("Stale storage receiver"))}
        let slot=handle.slot as usize;
        if self.state.borrow().storage.as_ref().is_none_or(|storage|slot>=storage.objects.len()){
            return Err(Throw::type_error("Invalid storage slot"));
        }
        Ok(slot)
    }
    fn storage_wrapper(&self,slot:usize)->Result<JsValue,Throw>{
        self.state.borrow().storage.as_ref().and_then(|s|s.wrappers.get(&slot)).cloned()
            .ok_or_else(||Throw::type_error("Storage wrapper missing"))
    }
    fn storage_dom_error(&mut self,name:&str,message:&str)->Throw {
        let result=(||{
            let global=self.js.binding_global_object();
            let constructor=self.js.binding_get_property(&global,"DOMException")?;
            let value=self.js.binding_construct(&constructor,vec![JsValue::str(message),JsValue::str(name)])?;
            Ok::<Throw,Throw>(self.js.binding_throw(value))
        })();
        result.unwrap_or_else(|_|Throw::type_error(format!("{name}: {message}")))
    }
    fn storage_string(&mut self,value:&JsValue)->Result<String,Throw>{
        Ok(String::from_utf16_lossy(&self.js.binding_to_dom_string(value)?))
    }
    fn storage_arg<'a>(&self,args:&'a[JsValue],index:usize)->Result<&'a JsValue,Throw>{
        args.get(index).ok_or_else(||Throw::type_error("Missing required argument"))
    }
    fn storage_schedule(&mut self,job:StorageJob)->Result<(),Throw>{
        let needs_tick={
            let mut state=self.state.borrow_mut();let storage=state.storage.as_mut().expect("storage initialized");
            storage.pending.push_back(job);
            if storage.tick_scheduled {false}else{storage.tick_scheduled=true;true}
        };
        if needs_tick {
            let callback=self.native("storage task",0,NativeEntry::StorageTick,HostCallPolicy::CallOnly);
            self.js.schedule_timer(callback,0,Vec::new())?;
        }
        Ok(())
    }
}

impl RuntimeBindingHost<'_> {
    fn storage_key_for_url(url:&str)->Result<StorageKey,Throw>{
        use amazing_fetch::{HttpUrl,Site,psl::PublicSuffixList};
        let parsed=HttpUrl::parse(url).map_err(|_|Throw::type_error("Storage requires an HTTP(S) origin"))?;
        let origin=parsed.origin();
        let site=match origin.site(&PublicSuffixList) {
            Site::Tuple {scheme,domain}=>format!("{scheme}://{domain}"),
            Site::Opaque(_)=>return Err(Throw::type_error("Storage is unavailable for opaque origins")),
        };
        Ok(StorageKey::new(origin.serialize(),site))
    }
    fn storage_area_session(&self,slot:usize)->Result<bool,Throw>{
        match self.state.borrow().storage.as_ref().and_then(|s|s.objects.get(slot)) {
            Some(StorageObject::Area {session})=>Ok(*session),
            _=>Err(Throw::type_error("Expected Storage")),
        }
    }
    fn storage_area_pairs(&mut self,slot:usize)->Result<Vec<(Vec<u16>,Vec<u16>)>,Throw>{
        let session=self.storage_area_session(slot)?;
        let key=self.storage_key()?;let bucket=BucketKey::new(key.clone(),"default");
        let id=self.state.borrow().arena;
        let web=self.state.borrow().storage.as_ref().expect("storage initialized").web.clone();
        let mut web=web.borrow_mut();
        let area=if session {Some(&*web.session(id,key))} else {web.local(&bucket)};
        let mut pairs=Vec::new();
        if let Some(area)=area {for index in 0..area.len(){let name=area.key(index).unwrap().to_vec();let value=area.get(&name).unwrap().to_vec();pairs.push((name,value));}}
        Ok(pairs)
    }
    fn storage_area_get(&mut self,slot:usize,name:&[u16])->Result<Option<Vec<u16>>,Throw>{
        Ok(self.storage_area_pairs(slot)?.into_iter().find(|(key,_)|key==name).map(|(_,value)|value))
    }
    fn storage_area_change(&mut self,slot:usize,name:Option<Vec<u16>>,value:Option<Vec<u16>>)->Result<(),Throw>{
        let session=self.storage_area_session(slot)?;
        let key=self.storage_key()?;let bucket=BucketKey::new(key.clone(),"default");
        let id=self.state.borrow().arena;
        let result={
            let web=self.state.borrow().storage.as_ref().expect("storage initialized").web.clone();
            let mut web=web.borrow_mut();
            if session {
                let area=web.session(id,key.clone());
                match (name,value){(Some(name),Some(value))=>area.set(name,value,id).map_err(|e|format!("{e:?}")),
                    (Some(name),None)=>Ok(area.remove(&name,id)),(None,_)=>Ok(area.clear(id))}
            } else {
                web.change_local(bucket,|area|match (name,value){
                    (Some(name),Some(value))=>area.set(name,value,id),
                    (Some(name),None)=>Ok(area.remove(&name,id)),(None,_)=>Ok(area.clear(id)),
                }).map_err(|e|format!("{e:?}"))
            }
        };
        match result {
            Ok(Some(change)) if !session=>{
                let url=self.state.borrow().document_url.clone().unwrap_or_default();
                let path=self.state.borrow().storage.as_ref().unwrap()._test_root.as_ref()
                    .map(|root|root.path.clone()).unwrap_or_else(Self::storage_profile_path);
                STORAGE_DOCUMENTS.with(|documents| {
                    let mut documents=documents.borrow_mut();
                    if let Some(peers)=documents.get_mut(&path) {
                        peers.retain(|peer| peer.upgrade().is_some());
                        for peer in peers.iter().filter_map(Weak::upgrade) {
                            let recipient_url=peer.borrow().document_url.clone();
                            if peer.borrow().arena==change.source ||
                                !recipient_url.as_deref().is_some_and(|recipient|
                                    Self::storage_key_for_url(recipient).ok().as_ref()==Some(&key)) {continue}
                            peer.borrow_mut().storage_inbox.push_back(StorageEventDelivery {
                                change: change.clone(), url: url.clone(),
                            });
                        }
                    }
                });
            },
            Ok(Some(_))=>{},
            Ok(None)=>{},
            Err(error) if error.contains("QuotaExceeded")=>return Err(self.storage_dom_error("QuotaExceededError","Storage quota exceeded")),
            Err(error)=>return Err(Throw::type_error(format!("Storage failure: {error}"))),
        }
        Ok(())
    }
    fn install_storage_legacy(&mut self){
        let brand=STORAGE_BRAND_BASE+SI::Storage as u64;
        let state=self.state.clone();
        let names=Rc::new(move |handle:HostHandle|{
            let borrowed=state.borrow();
            if handle.arena!=borrowed.arena||handle.brand!=brand{return Err(Throw::type_error("Invalid Storage handle"))}
            let url=borrowed.document_url.clone().ok_or_else(||Throw::type_error("Storage requires a document URL"))?;
            let key=Self::storage_key_for_url(&url)?;let bucket=BucketKey::new(key.clone(),"default");
            let storage=borrowed.storage.as_ref().ok_or_else(||Throw::type_error("Storage unavailable"))?;
            let session=matches!(storage.objects.get(handle.slot as usize),Some(StorageObject::Area {session:true}));
            let web=storage.web.clone();
            drop(borrowed);
            let mut web=web.borrow_mut();
            let area=if session {&*web.session(handle.arena,key)} else {match web.local(&bucket) {Some(area)=>area,None=>return Ok(Vec::new())}};
            Ok((0..area.len()).map(|i|area.key(i).unwrap().to_vec()).collect())
        });
        let state=self.state.clone();
        let get=Rc::new(move |js:&mut Interpreter,handle:HostHandle,name:&[u16]|{
            let mut adapter=RuntimeBindingHost {js,state:state.clone()};
            let value=adapter.storage_area_get(handle.slot as usize,name)?;
            Ok(value.map_or(JsValue::Null,JsValue::str_units))
        });
        let state=self.state.clone();
        let set=Rc::new(move |js:&mut Interpreter,handle:HostHandle,name:&[u16],value:JsValue|{
            let mut adapter=RuntimeBindingHost {js,state:state.clone()};
            let value=adapter.js.binding_to_dom_string(&value)?;
            adapter.storage_area_change(handle.slot as usize,Some(name.to_vec()),Some(value))
        });
        let state=self.state.clone();
        let delete=Rc::new(move |js:&mut Interpreter,handle:HostHandle,name:&[u16]|{
            let mut adapter=RuntimeBindingHost {js,state:state.clone()};
            adapter.storage_area_change(handle.slot as usize,Some(name.to_vec()),None)?;Ok(true)
        });
        self.js.binding_define_legacy_platform_object(brand,RuntimeLegacyHooks {
            supported_index_count:Rc::new(|_|Ok(0)),supported_names:Some(names),
            indexed_getter:Rc::new(|_,_,_|Ok(JsValue::Undefined)),named_getter:Some(get),
            indexed_setter:None,named_setter:Some(set),named_deleter:Some(delete),
            unenumerable_names:false,override_builtins:false,
        });
    }
    pub(super) fn storage_invoke(&mut self,interface:SI,member:&'static str,kind:SM,this:JsValue,args:&[JsValue])->Result<JsValue,Throw>{
        if member=="!singleton" {
            return self.storage_singleton(interface);
        }
        match interface {
            SI::Storage=>self.storage_area_invoke(member,kind,this,args),
            SI::IDBFactory=>self.storage_factory_invoke(member,this,args),
            SI::IDBKeyRange=>self.storage_range_invoke(member,kind,this,args),
            SI::IDBVersionChangeEvent=>{
                let name=self.storage_string(self.storage_arg(args,0)?)?;
                let options=args.get(1).cloned().unwrap_or(JsValue::Undefined);
                let old=if matches!(options,JsValue::Object(_)){
                    let value=self.js.binding_get_property(&options,"oldVersion")?;
                    if matches!(value,JsValue::Undefined){0}else{self.js.binding_to_number(&value)? as u64}
                }else{0};
                let new=if matches!(options,JsValue::Object(_)){
                    let value=self.js.binding_get_property(&options,"newVersion")?;
                    if matches!(value,JsValue::Undefined|JsValue::Null){None}else{Some(self.js.binding_to_number(&value)? as u64)}
                }else{None};
                self.storage_version_event(&name,old,new)
            },
            SI::CacheStorage|SI::Cache=>self.storage_cache_invoke(interface,member,this,args),
            SI::StorageManager=>self.storage_manager_invoke(member,this),
            SI::StorageBucketManager=>self.storage_bucket_manager_invoke(member,this,args),
            SI::StorageBucket=>self.storage_bucket_invoke(member,kind,this,args),
            SI::LockManager=>self.storage_lock_manager_invoke(member,this,args),
            SI::Lock=>self.storage_lock_invoke(member,this),
            _=>self.storage_idb_invoke(interface,member,kind,this,args),
        }
    }
    fn storage_area_invoke(&mut self,member:&str,_kind:SM,this:JsValue,args:&[JsValue])->Result<JsValue,Throw>{
        if member=="localStorage"||member=="sessionStorage" {
            let _=self.storage_key()?;self.storage_ensure()?;
            let session=member=="sessionStorage";
            let existing={let state=self.state.borrow();let storage=state.storage.as_ref().unwrap();
                storage.objects.iter().position(|o|matches!(o,StorageObject::Area{session:s} if *s==session))};
            if let Some(slot)=existing{return self.storage_wrapper(slot)}
            return self.storage_allocate(SI::Storage,StorageObject::Area{session});
        }
        let slot=self.storage_slot(&this,SI::Storage)?;
        match member {
            "length"=>Ok(JsValue::num(self.storage_area_pairs(slot)?.len() as f64)),
            "key"=>{
                let number=self.js.binding_to_number(self.storage_arg(args,0)?)?;
                let index=if number.is_finite()&&number>=0.0{number as usize}else{usize::MAX};
                Ok(self.storage_area_pairs(slot)?.get(index).map_or(JsValue::Null,|(name,_)|JsValue::str_units(name.clone())))
            },
            "getItem"=>{let key=self.js.binding_to_dom_string(self.storage_arg(args,0)?)?;
                Ok(self.storage_area_get(slot,&key)?.map_or(JsValue::Null,JsValue::str_units))},
            "setItem"=>{let key=self.js.binding_to_dom_string(self.storage_arg(args,0)?)?;
                let value=self.js.binding_to_dom_string(self.storage_arg(args,1)?)?;
                self.storage_area_change(slot,Some(key),Some(value))?;Ok(JsValue::Undefined)},
            "removeItem"=>{let key=self.js.binding_to_dom_string(self.storage_arg(args,0)?)?;
                self.storage_area_change(slot,Some(key),None)?;Ok(JsValue::Undefined)},
            "clear"=>{self.storage_area_change(slot,None,None)?;Ok(JsValue::Undefined)},
            _=>Err(Throw::type_error("Unknown Storage member")),
        }
    }
}

impl RuntimeBindingHost<'_> {
    fn storage_lock_invoke(&mut self,member:&str,this:JsValue)->Result<JsValue,Throw>{
        let slot=self.storage_slot(&this,SI::Lock)?;
        let (name,mode)={let state=self.state.borrow();match &state.storage.as_ref().unwrap().objects[slot]{
            StorageObject::Lock{name,mode}=>(name.clone(),*mode),_=>unreachable!()}};
        match member {"name"=>Ok(JsValue::str(name)),"mode"=>Ok(JsValue::str(match mode{LockMode::Shared=>"shared",LockMode::Exclusive=>"exclusive"})),
            _=>Err(Throw::type_error("Unknown Lock member"))}
    }
    fn storage_lock_manager_invoke(&mut self,member:&str,this:JsValue,args:&[JsValue])->Result<JsValue,Throw>{
        self.storage_slot(&this,SI::LockManager)?;
        let key=self.storage_key()?;
        if member=="query" {
            let locks=self.state.borrow().storage.as_ref().unwrap().locks.clone();
            let snapshot=locks.borrow().query(&key);
            let result=self.js.binding_create_object(None)?;
            for (name,items) in [("held",snapshot.held),("pending",snapshot.pending)] {
                let mut values=Vec::new();
                for item in items {
                    let value=self.js.binding_create_object(None)?;
                    self.define_data_property(&value,"name",JsValue::str(item.name),true,true,true)?;
                    self.define_data_property(&value,"mode",JsValue::str(match item.mode{LockMode::Shared=>"shared",LockMode::Exclusive=>"exclusive"}),true,true,true)?;
                    self.define_data_property(&value,"clientId",JsValue::str(item.client_id.to_string()),true,true,true)?;
                    values.push(value);
                }
                let values=self.js.binding_create_array(values)?;
                self.define_data_property(&result,name,values,true,true,true)?;
            }
            let promise=self.new_promise()?;self.resolve_promise(&promise,result)?;return Ok(promise)
        }
        if member!="request"{return Err(Throw::type_error("Unknown LockManager member"))}
        let name=self.storage_string(self.storage_arg(args,0)?)?;
        let (options,callback)=if args.get(1).is_some_and(|value|self.js.binding_is_callable(value)) {
            (None,args.get(1).cloned())
        }else{(args.get(1),args.get(2).cloned())};
        let callback=callback.filter(|value|self.js.binding_is_callable(value))
            .ok_or_else(||Throw::type_error("Lock callback must be callable"))?;
        let mut mode=LockMode::Exclusive;let mut flags=LockOptions::default();
        if let Some(JsValue::Object(_))=options {
            let value=self.js.binding_get_property(options.unwrap(),"mode")?;
            if !matches!(value,JsValue::Undefined){mode=match self.storage_string(&value)?.as_str(){
                "shared"=>LockMode::Shared,"exclusive"=>LockMode::Exclusive,_=>return Err(Throw::type_error("Invalid lock mode"))}}
            let value=self.js.binding_get_property(options.unwrap(),"ifAvailable")?;flags.if_available=self.js.binding_to_boolean(&value);
            let value=self.js.binding_get_property(options.unwrap(),"steal")?;flags.steal=self.js.binding_to_boolean(&value);
        }
        let promise=self.new_promise()?;
        let (locks,client_id)={let state=self.state.borrow();let storage=state.storage.as_ref().unwrap();(storage.locks.clone(),storage.lock_client_id)};
        let outcome=locks.borrow_mut().request(key.clone(),client_id,name.clone(),mode,flags);
        let outcome=match outcome {Ok(value)=>value,Err(error)=>{
            let name=match error{amazing_storage::locks::Error::InvalidName=>"NotSupportedError",amazing_storage::locks::Error::IncompatibleOptions=>"NotSupportedError"};
            let error=self.storage_dom_error(name,"Invalid lock request");self.reject_promise(&promise,error.0)?;return Ok(promise)
        }};
        static NEXT_UNAVAILABLE:AtomicU64=AtomicU64::new(1u64<<63);
        let id=outcome.id.unwrap_or_else(||NEXT_UNAVAILABLE.fetch_add(1,Ordering::Relaxed));
        let acquired=outcome.granted.contains(&id);
        let started=acquired||outcome.id.is_none();
        self.state.borrow_mut().storage.as_mut().unwrap().lock_callbacks.insert(id,
            LockCallback{callback,promise:promise.clone(),key,name,mode,started,acquired});
        if started {self.storage_schedule(StorageJob::LockGranted{id})?;}
        Ok(promise)
    }
    fn storage_lock_granted(&mut self,id:u64)->Result<(),Throw>{
        let pending={let state=self.state.borrow();state.storage.as_ref().unwrap().lock_callbacks.get(&id)
            .map(|pending|(pending.callback.clone(),pending.name.clone(),pending.mode,pending.acquired))};
        let Some((callback,name,mode,acquired))=pending else{return Ok(())};
        let lock=if acquired {self.storage_allocate(SI::Lock,StorageObject::Lock{name,mode})?}else{JsValue::Null};
        let result=(||->Result<(),Throw>{
            let returned=binding_call(self.js,&callback,&JsValue::Undefined,&[lock])?;
            let global=self.js.binding_global_object();
            let constructor=self.js.binding_get_property(&global,"Promise")?;
            let resolve=self.js.binding_get_property(&constructor,"resolve")?;
            let pending=binding_call(self.js,&resolve,&constructor,&[returned])?;
            let success=self.native("lock callback fulfilled",1,NativeEntry::StorageLockSettled(id,true),HostCallPolicy::CallOnly);
            let failure=self.native("lock callback rejected",1,NativeEntry::StorageLockSettled(id,false),HostCallPolicy::CallOnly);
            let then=self.js.binding_get_property(&pending,"then")?;
            binding_call(self.js,&then,&pending,&[success,failure])?;
            Ok(())
        })();
        if let Err(error)=result {self.storage_lock_finish(id,Err(error.0))?;}
        Ok(())
    }
    pub(super) fn storage_lock_settled(&mut self,id:u64,success:bool,args:&[JsValue])->Result<JsValue,Throw>{
        let value=args.first().cloned().unwrap_or(JsValue::Undefined);
        self.storage_lock_finish(id,if success{Ok(value)}else{Err(value)})?;
        Ok(JsValue::Undefined)
    }
    fn storage_lock_finish(&mut self,id:u64,result:Result<JsValue,JsValue>)->Result<(),Throw>{
        let pending=self.state.borrow_mut().storage.as_mut().unwrap().lock_callbacks.remove(&id);
        let Some(pending)=pending else{return Ok(())};
        if pending.acquired {self.state.borrow().storage.as_ref().unwrap().locks.borrow_mut().release(id);}
        match result {Ok(value)=>self.resolve_promise(&pending.promise,value)?,Err(error)=>self.reject_promise(&pending.promise,error)?}
        Ok(())
    }
}

impl RuntimeBindingHost<'_> {
    fn storage_manager_invoke(&mut self,member:&str,this:JsValue)->Result<JsValue,Throw>{
        self.storage_slot(&this,SI::StorageManager)?;
        let promise=self.new_promise()?;
        let result=(||->Result<JsValue,Throw>{
            let key=self.storage_key()?;
            let manager=self.state.borrow().storage.as_ref().unwrap().manager.clone();
            match member {
                "estimate"=>{
                    let estimate=manager.borrow().estimate(&key);
                    let value=self.js.binding_create_object(None)?;
                    self.define_data_property(&value,"usage",JsValue::num(estimate.usage as f64),true,true,true)?;
                    self.define_data_property(&value,"quota",JsValue::num(estimate.quota as f64),true,true,true)?;
                    Ok(value)
                },
                "persisted"=>Ok(JsValue::Bool(manager.borrow().persisted(&key))),
                "persist"=>{let granted=manager.borrow_mut().persist(key)
                    .map_err(|error|Throw::type_error(format!("StorageManager persist: {error:?}")))?;
                    Ok(JsValue::Bool(granted))},
                _=>Err(Throw::type_error("Unknown StorageManager member")),
            }
        })();
        match result {Ok(value)=>self.resolve_promise(&promise,value)?,Err(error)=>self.reject_promise(&promise,error.0)?}
        Ok(promise)
    }
}

impl RuntimeBindingHost<'_> {
    fn storage_bucket_manager_invoke(&mut self, member:&str, this:JsValue, args:&[JsValue])->Result<JsValue,Throw>{
        self.storage_slot(&this,SI::StorageBucketManager)?;
        let promise=self.new_promise()?;
        let result=(||->Result<JsValue,Throw>{
            let key=self.storage_key()?;
            match member {
                "keys"=>{
                    let names=self.state.borrow().storage.as_ref().unwrap().buckets.borrow().keys(&key);
                    let values=names.into_iter().map(JsValue::str).collect();
                    Ok(self.js.binding_create_array(values)?)
                },
                "open"=>{
                    let name=self.storage_string(self.storage_arg(args,0)?)?;
                    if !valid_bucket_name(&name){return Err(Throw::type_error("Invalid bucket name"))}
                    let mut policy=Policy::default();
                    if let Some(options@JsValue::Object(_))=args.get(1){
                        let quota=self.js.binding_get_property(options,"quota")?;
                        if !matches!(quota,JsValue::Undefined){
                            let number=self.js.binding_to_number(&quota)?;
                            if !number.is_finite()||number<1.0||number>9_007_199_254_740_991.0{
                                return Err(Throw::type_error("Invalid bucket quota"))}
                            policy.quota=Some(number as u64);
                        }
                        let durability=self.js.binding_get_property(options,"durability")?;
                        if !matches!(durability,JsValue::Undefined){
                            policy.durability=match self.storage_string(&durability)?.as_str(){
                                "strict"=>Durability::Strict,"relaxed"=>Durability::Relaxed,
                                _=>return Err(Throw::type_error("Invalid bucket durability"))};
                        }
                        let expires=self.js.binding_get_property(options,"expires")?;
                        if !matches!(expires,JsValue::Undefined|JsValue::Null){
                            let number=self.js.binding_to_number(&expires)?;
                            if !number.is_finite(){return Err(Throw::type_error("Invalid bucket expires"))}
                            policy.expires=Some(number as i64);
                        }
                    }
                    let epoch=self.state.borrow().storage.as_ref().unwrap().buckets.borrow_mut()
                        .open(key,name.clone(),policy)
                        .map_err(|error|Throw::type_error(format!("Bucket open: {error:?}")))?;
                    self.storage_allocate(SI::StorageBucket,StorageObject::Bucket{name,epoch})
                },
                "delete"=>{
                    let name=self.storage_string(self.storage_arg(args,0)?)?;
                    if !valid_bucket_name(&name){return Err(Throw::type_error("Invalid bucket name"))}
                    self.state.borrow().storage.as_ref().unwrap().buckets.borrow_mut()
                        .delete(&key,&name)
                        .map_err(|error|Throw::type_error(format!("Bucket delete: {error:?}")))?;
                    Ok(JsValue::Undefined)
                },
                _=>Err(Throw::type_error("Unknown StorageBucketManager member")),
            }
        })();
        match result {Ok(value)=>self.resolve_promise(&promise,value)?,Err(error)=>self.reject_promise(&promise,error.0)?}
        Ok(promise)
    }
    fn storage_bucket_invoke(&mut self, member:&str, kind:SM, this:JsValue, args:&[JsValue])->Result<JsValue,Throw>{
        let slot=self.storage_slot(&this,SI::StorageBucket)?;
        let (name,epoch)={let state=self.state.borrow();match &state.storage.as_ref().unwrap().objects[slot]{
            StorageObject::Bucket{name,epoch}=>(name.clone(),*epoch),_=>unreachable!()}};
        if kind!=SM::Getter {
            let key=self.storage_key()?;
            let promise=self.new_promise()?;
            let result=(||->Result<JsValue,Throw>{
                if !self.state.borrow().storage.as_ref().unwrap().buckets.borrow().handle_valid(&key,&name,epoch){
                    return Err(self.storage_dom_error("UnknownError","Bucket has been deleted"))}
                let buckets=self.state.borrow().storage.as_ref().unwrap().buckets.clone();
                match member {
                    "persisted"=>Ok(JsValue::Bool(buckets.borrow().policy(&key,&name).is_some_and(|policy|policy.persisted))),
                    "persist"=>{
                        let mut policy=buckets.borrow().policy(&key,&name).cloned().unwrap_or_default();
                        policy.persisted=true;
                        buckets.borrow_mut().update(&key,&name,policy)
                            .map_err(|error|Throw::type_error(format!("Bucket persist: {error:?}")))?;
                        Ok(JsValue::Bool(true))
                    },
                    "durability"=>Ok(JsValue::str(match buckets.borrow().policy(&key,&name).map(|policy|policy.durability){
                        Some(Durability::Strict)=>"strict",_=>"relaxed"})),
                    "expires"=>Ok(buckets.borrow().policy(&key,&name).and_then(|policy|policy.expires)
                        .map_or(JsValue::Null,|expires|JsValue::num(expires as f64))),
                    "setExpires"=>{
                        let number=self.js.binding_to_number(self.storage_arg(args,0)?)?;
                        if !number.is_finite(){return Err(Throw::type_error("Invalid expires"))}
                        let mut policy=buckets.borrow().policy(&key,&name).cloned().unwrap_or_default();
                        policy.expires=Some(number as i64);
                        buckets.borrow_mut().update(&key,&name,policy)
                            .map_err(|error|Throw::type_error(format!("Bucket setExpires: {error:?}")))?;
                        Ok(JsValue::Undefined)
                    },
                    "estimate"=>{
                        let policy=buckets.borrow().policy(&key,&name).cloned().unwrap_or_default();
                        let quota=policy.quota.unwrap_or_else(||{
                            let manager=self.state.borrow().storage.as_ref().unwrap().manager.clone();
                            manager.borrow().estimate(&key).quota});
                        let value=self.js.binding_create_object(None)?;
                        self.define_data_property(&value,"usage",JsValue::num(0.0),true,true,true)?;
                        self.define_data_property(&value,"quota",JsValue::num(quota as f64),true,true,true)?;
                        Ok(value)
                    },
                    _=>Err(Throw::type_error("Unknown StorageBucket member")),
                }
            })();
            match result {Ok(value)=>self.resolve_promise(&promise,value)?,Err(error)=>self.reject_promise(&promise,error.0)?}
            return Ok(promise)
        }
        match member {
            "name"=>Ok(JsValue::str(name)),
            "indexedDB"=>self.storage_allocate(SI::IDBFactory,StorageObject::Factory{bucket:name}),
            "caches"=>self.storage_allocate(SI::CacheStorage,StorageObject::CacheStorage{bucket:name}),
            _=>Err(Throw::type_error("Unknown StorageBucket member")),
        }
    }
}

impl RuntimeBindingHost<'_> {
    fn storage_idb_error(&mut self, error: idb::Error) -> Throw {
        let name = match error {
            idb::Error::Data => "DataError", idb::Error::Constraint => "ConstraintError",
            idb::Error::NotFound => "NotFoundError", idb::Error::InvalidState => "InvalidStateError",
            idb::Error::ReadOnly => "ReadOnlyError", idb::Error::TransactionInactive => "TransactionInactiveError",
            idb::Error::Abort => "AbortError", idb::Error::Version => "VersionError",
            idb::Error::Blocked => "InvalidStateError", idb::Error::QuotaExceeded => "QuotaExceededError",
        };
        self.storage_dom_error(name, name)
    }
    // For a lookup keyed off a handle the caller already holds (an
    // IDBObjectStore/IDBIndex JS object created earlier against a name that
    // existed then): a `NotFound` from the engine now means the underlying
    // store/index was removed since -- e.g. by `deleteObjectStore`/
    // `deleteIndex` in the same upgrade transaction -- which per spec throws
    // InvalidStateError, not NotFoundError (that's reserved for a fresh
    // by-name lookup, like `objectStore.index("name-that-was-never-there")`,
    // which should keep using `storage_idb_error` unchanged).
    fn storage_idb_error_for_handle(&mut self, error: idb::Error) -> Throw {
        match error {
            idb::Error::NotFound => self.storage_dom_error("InvalidStateError", "InvalidStateError"),
            other => self.storage_idb_error(other),
        }
    }
    fn storage_request(&mut self, open: bool, source: JsValue, transaction: JsValue) -> Result<(usize, JsValue), Throw> {
        let kind=if open {SI::IDBOpenDBRequest}else{SI::IDBRequest};
        let value=self.storage_allocate(kind,StorageObject::Request {result:JsValue::Undefined,error:JsValue::Null,
            ready:false,source,transaction,handlers:HashMap::new(),listeners:Vec::new()})?;
        let slot=self.storage_slot(&value,kind)?;
        Ok((slot,value))
    }
    fn storage_factory_invoke(&mut self, member: &str, this: JsValue, args: &[JsValue]) -> Result<JsValue, Throw> {
        let slot=self.storage_slot(&this,SI::IDBFactory)?;
        let bucket={let state=self.state.borrow();match &state.storage.as_ref().unwrap().objects[slot]{
            StorageObject::Factory{bucket}=>bucket.clone(),_=>unreachable!()}};
        let key=BucketKey::new(self.storage_key()?,bucket);
        match member {
            "cmp"=>{
                if args.len()<2 {
                    let global=self.js.binding_global_object();
                    let constructor=self.js.binding_get_property(&global,"TypeError")?;
                    let error=self.js.binding_construct(&constructor,vec![JsValue::str("Missing required argument")])?;
                    return Err(self.js.binding_throw(error));
                }
                let a=self.storage_to_key(self.storage_arg(args,0)?)?;
                let b=self.storage_to_key(self.storage_arg(args,1)?)?;
                Ok(JsValue::num(a.cmp(&b) as i32 as f64))
            },
            "databases"=>{
                let names=self.state.borrow().storage.as_ref().unwrap().idb.borrow().databases(&key);
                let mut entries=Vec::new();
                for (name,version) in names {
                    let entry=self.js.binding_create_object(None)?;
                    self.define_data_property(&entry,"name",JsValue::str(name),true,true,true)?;
                    self.define_data_property(&entry,"version",JsValue::num(version as f64),true,true,true)?;
                    entries.push(entry);
                }
                let array=self.js.binding_create_array(entries)?;
                let promise=self.new_promise()?;
                self.resolve_promise(&promise,array)?;
                Ok(promise)
            },
            "open"=>{
                let name=self.storage_string(self.storage_arg(args,0)?)?;
                let version=if let Some(value)=args.get(1) {Some(self.js.binding_to_number(value)? as u64)}else{None};
                if version==Some(0){return Err(Throw::type_error("Database version must be positive"))}
                let (request,wrapper)=self.storage_request(true,JsValue::Null,JsValue::Null)?;
                let result=self.state.borrow().storage.as_ref().unwrap().idb.borrow_mut().open(key.clone(),name.clone(),version);
                match result {
                    Ok(open)=>{
                        if open.blocked {self.state.borrow_mut().storage.as_mut().unwrap().blocked_opens.push_back((key.clone(),name.clone(),request));}
                        for event in open.events {
                            match event {
                                idb::OpenEvent::UpgradeNeeded{old,new}=>{
                                    self.storage_schedule(StorageJob::OpenUpgrade{request,connection:open.connection.unwrap(),name:name.clone(),key:key.clone(),old,new})?;
                                },
                                idb::OpenEvent::Blocked{old,new}=>{
                                    self.storage_schedule(StorageJob::OpenBlocked{request,old,new})?;
                                },
                                idb::OpenEvent::Success{connection,..}=>{
                                    let database=self.storage_allocate(SI::IDBDatabase,StorageObject::Database{connection,name:name.clone(),key:key.clone(),handlers:HashMap::new(),listeners:Vec::new()})?;
                                    self.storage_schedule(StorageJob::RequestSuccess{request,result:database})?;
                                },
                                idb::OpenEvent::VersionChange{connection,old,new}=>self.storage_schedule(StorageJob::DatabaseVersionChange{connection,old,new})?,
                            }
                        }
                    },
                    Err(error)=>{let error=self.storage_idb_error(error).0;self.storage_schedule(StorageJob::RequestError{request,error})?;},
                }
                Ok(wrapper)
            },
            "deleteDatabase"=>{
                let name=self.storage_string(self.storage_arg(args,0)?)?;
                let (request,wrapper)=self.storage_request(true,JsValue::Null,JsValue::Null)?;
                let old=self.state.borrow().storage.as_ref().unwrap().idb.borrow().database(&key,&name).map_or(0,|db|db.version);
                let connections={let state=self.state.borrow();let storage=state.storage.as_ref().unwrap();storage.objects.iter().filter_map(|object|match object{
                    StorageObject::Database{connection,name:db_name,..} if db_name==&name=>Some(*connection),_=>None}).collect::<Vec<_>>()};
                let result=self.state.borrow().storage.as_ref().unwrap().idb.borrow_mut().delete_database(&key,&name);
                match result {
                    Ok(_)=>self.storage_schedule(StorageJob::DeleteSuccess{request,old})?,
                    Err(idb::Error::Blocked)=>{
                        self.state.borrow_mut().storage.as_mut().unwrap().blocked_deletes.push_back((key,name,request));
                        for connection in connections {self.storage_schedule(StorageJob::DatabaseVersionChange{connection,old,new:None})?;}
                        self.storage_schedule(StorageJob::OpenBlocked{request,old,new:None})?;
                    },
                    Err(error)=>{let error=self.storage_idb_error(error).0;self.storage_schedule(StorageJob::RequestError{request,error})?;},
                }
                Ok(wrapper)
            },
            _=>Err(Throw::type_error("Unknown IDBFactory member")),
        }
    }
    fn storage_resume_blocked(&mut self)->Result<(),Throw>{
        let idb=self.state.borrow().storage.as_ref().unwrap().idb.clone();
        let resumed=idb.borrow_mut().resume_pending();
        for open in resumed {
            let Some(connection)=open.connection else{continue};
            let (key,name)={let idb=idb.borrow();let Some(info)=idb.connection(connection) else{continue};(info.key.clone(),info.name.clone())};
            let request={let mut state=self.state.borrow_mut();let storage=state.storage.as_mut().unwrap();
                storage.blocked_opens.iter().position(|(pending_key,pending_name,_)|*pending_key==key&&*pending_name==name)
                    .and_then(|position|storage.blocked_opens.remove(position)).map(|(_,_,request)|request)};
            if let Some(request)=request {
                for event in open.events {
                    if let idb::OpenEvent::UpgradeNeeded{old,new}=event {
                        self.storage_schedule(StorageJob::OpenUpgrade{request,connection,name:name.clone(),key:key.clone(),old,new})?;
                    }
                }
            }
        }
        let pending={let mut state=self.state.borrow_mut();std::mem::take(&mut state.storage.as_mut().unwrap().blocked_deletes)};
        for (key,name,request) in pending {
            match idb.borrow_mut().delete_database(&key,&name) {
                Ok(old)=>self.storage_schedule(StorageJob::DeleteSuccess{request,old})?,
                Err(idb::Error::Blocked)=>self.state.borrow_mut().storage.as_mut().unwrap().blocked_deletes.push_back((key,name,request)),
                Err(error)=>{let error=self.storage_idb_error(error).0;self.storage_schedule(StorageJob::RequestError{request,error})?;},
            }
        }
        Ok(())
    }
    fn storage_range_invoke(&mut self, member: &str, kind: SM, this: JsValue, args: &[JsValue]) -> Result<JsValue, Throw> {
        if kind==SM::StaticMethod {
            let a=self.storage_to_key(self.storage_arg(args,0)?)?;
            let range=match member {
                "only"=>KeyRange::only(a),
                // A one-sided range's open end is unbounded, which counts as
                // open per the IDBKeyRange.lowerBound()/upperBound() spec steps
                // (idbkeyrange.any.js asserts upperOpen/lowerOpen are true on
                // the unbounded side, not left at their struct default).
                "lowerBound"=>KeyRange{lower:Some(a),upper:None,lower_open:args.get(1).is_some_and(|v|self.js.binding_to_boolean(v)),upper_open:true},
                "upperBound"=>KeyRange{lower:None,upper:Some(a),lower_open:true,upper_open:args.get(1).is_some_and(|v|self.js.binding_to_boolean(v))},
                "bound"=>{let b=self.storage_to_key(self.storage_arg(args,1)?)?;
                    KeyRange::bound(a,b,args.get(2).is_some_and(|v|self.js.binding_to_boolean(v)),
                        args.get(3).is_some_and(|v|self.js.binding_to_boolean(v))).map_err(|e|self.storage_idb_error(e))?},
                _=>return Err(Throw::type_error("Unknown IDBKeyRange member")),
            };
            return self.storage_allocate(SI::IDBKeyRange,StorageObject::Range(range));
        }
        let slot=self.storage_slot(&this,SI::IDBKeyRange)?;
        let range=match self.state.borrow().storage.as_ref().unwrap().objects.get(slot){Some(StorageObject::Range(range))=>range.clone(),_=>unreachable!()};
        match member {
            "lower"=>range.lower.as_ref().map_or(Ok(JsValue::Undefined),|v|self.storage_from_key(v)),
            "upper"=>range.upper.as_ref().map_or(Ok(JsValue::Undefined),|v|self.storage_from_key(v)),
            "lowerOpen"=>Ok(JsValue::Bool(range.lower_open)),"upperOpen"=>Ok(JsValue::Bool(range.upper_open)),
            "includes"=>Ok(JsValue::Bool(range.includes(&self.storage_to_key(self.storage_arg(args,0)?)?))),
            _=>Err(Throw::type_error("Unknown IDBKeyRange member")),
        }
    }
    fn storage_cache_invoke(&mut self, _interface: SI, _member: &str, _this: JsValue, _args: &[JsValue]) -> Result<JsValue, Throw> {
        Err(Throw::type_error("Cache API awaits Fetch wrappers"))
    }
    fn storage_idb_invoke(&mut self, interface: SI, member: &str, kind: SM, this: JsValue, args: &[JsValue]) -> Result<JsValue, Throw> {
        if matches!(member,"addEventListener"|"removeEventListener") {
            let slot=self.storage_slot(&this,interface)?;
            let name=self.storage_string(self.storage_arg(args,0)?)?;
            let callback=self.storage_arg(args,1)?.clone();
            let mut state=self.state.borrow_mut();
            let listeners=match &mut state.storage.as_mut().unwrap().objects[slot] {
                StorageObject::Request{listeners,..}|StorageObject::Transaction{listeners,..}|StorageObject::Database{listeners,..}=>listeners,
                _=>return Err(Throw::type_error("Not an event target")),
            };
            if member=="addEventListener" {listeners.push((name,callback));}
            else {listeners.retain(|(event,listener)|event!=&name||!self.js.binding_same_value(listener,&callback));}
            return Ok(JsValue::Undefined)
        }
        match interface {
            SI::IDBRequest|SI::IDBOpenDBRequest=>{
                let slot=self.storage_slot(&this,interface)?;
                let (result,error,ready,source,transaction,handler)={let state=self.state.borrow();
                    match &state.storage.as_ref().unwrap().objects[slot] {
                        StorageObject::Request{result,error,ready,source,transaction,handlers,..}=>(result.clone(),error.clone(),*ready,source.clone(),transaction.clone(),handlers.get(member.trim_start_matches("on")).cloned()),
                        _=>unreachable!(),
                    }};
                if member.starts_with("on") {
                    if kind==SM::Setter {let callback=args.first().cloned().unwrap_or(JsValue::Null);
                        if let StorageObject::Request{handlers,..}=&mut self.state.borrow_mut().storage.as_mut().unwrap().objects[slot]{handlers.insert(member[2..].to_owned(),callback);}
                        return Ok(JsValue::Undefined)}
                    return Ok(handler.unwrap_or(JsValue::Null))
                }
                match member {"result"=>if ready{Ok(result)}else{Err(self.storage_dom_error("InvalidStateError","Request is pending"))},
                    "error"=>if ready {Ok(error)}else{Err(self.storage_dom_error("InvalidStateError","Request is pending"))},
                    "source"=>Ok(source),"transaction"=>Ok(transaction),
                    "readyState"=>Ok(JsValue::str(if ready{"done"}else{"pending"})),
                    _=>Err(Throw::type_error("Unknown IDBRequest member")),}
            },
            SI::IDBDatabase=>{
                let slot=self.storage_slot(&this,interface)?;
                if member.starts_with("on") {
                    if kind==SM::Setter {let callback=args.first().cloned().unwrap_or(JsValue::Null);
                        if let StorageObject::Database{handlers,..}=&mut self.state.borrow_mut().storage.as_mut().unwrap().objects[slot]{handlers.insert(member[2..].to_owned(),callback);}
                        return Ok(JsValue::Undefined)}
                    let state=self.state.borrow();if let StorageObject::Database{handlers,..}=&state.storage.as_ref().unwrap().objects[slot]{return Ok(handlers.get(&member[2..]).cloned().unwrap_or(JsValue::Null))}
                }
                // `key` comes from the cached field on StorageObject::Database (set when the
                // connection was opened), not from a live `idb::Connection` lookup: the latter
                // throws once the connection is closed, which broke every member access
                // (including the spec-idempotent second `close()` call) after a database's
                // first close. See `indexeddb_database_members_survive_close` below.
                let (connection,name,key)={let state=self.state.borrow();match &state.storage.as_ref().unwrap().objects[slot]{
                    StorageObject::Database{connection,name,key,..}=>(*connection,name.clone(),key.clone()),_=>unreachable!()}};
                match member {
                    "name"=>Ok(JsValue::str(name)),
                    "version"=>{let state=self.state.borrow();let version=state.storage.as_ref().unwrap().idb.borrow().connection(connection).map_or(0,|connection|connection.version);
                        Ok(JsValue::num(version as f64))},
                    "objectStoreNames"=>{let names={let state=self.state.borrow();let storage=state.storage.as_ref().unwrap();
                        storage.objects.iter().find_map(|object|match object {StorageObject::Transaction{connection:id,mode:Mode::VersionChange,value:Some(tx),..} if *id==connection=>
                            Some(tx.database().store_names().into_iter().map(str::to_owned).collect::<Vec<_>>()),_=>None})
                            .or_else(||storage.idb.borrow().database(&key,&name).map(|db|db.store_names().into_iter().map(str::to_owned).collect::<Vec<_>>())).unwrap_or_default()};
                        self.js.binding_create_array(names.into_iter().map(JsValue::str).collect())},
                    "close"=>{let result=self.state.borrow().storage.as_ref().unwrap().idb.borrow_mut().close(connection);
                        if let Err(error)=result {if error!=idb::Error::NotFound{return Err(self.storage_idb_error(error))}}
                        self.storage_resume_blocked()?;Ok(JsValue::Undefined)},
                    "transaction"=>{
                        let names_value=self.storage_arg(args,0)?.clone();
                        let names=if let JsValue::Object(object)=&names_value {if matches!(object.borrow().exotic,Exotic::Array){
                            let length=self.js.binding_get_property(&names_value,"length")?;let length=self.js.binding_to_number(&length)? as usize;
                            let mut names=Vec::new();for index in 0..length {let value=self.js.binding_get_property(&names_value,&index.to_string())?;names.push(self.storage_string(&value)?);}names
                        }else{vec![self.storage_string(&names_value)?]}}else{vec![self.storage_string(&names_value)?]};
                        let mode=if args.get(1).is_some_and(|v|matches!(v,JsValue::Str(s) if s.to_utf8_lossy()=="readwrite")){Mode::ReadWrite}else{Mode::ReadOnly};
                        self.storage_new_transaction(connection,names,mode)
                    },
                    "createObjectStore"=>{
                        let name=self.storage_string(self.storage_arg(args,0)?)?;
                        let options=args.get(1).cloned().unwrap_or(JsValue::Undefined);
                        let key_path=if matches!(options,JsValue::Object(_)) {let path=self.js.binding_get_property(&options,"keyPath")?;
                            self.storage_parse_key_path(&path)?}else{None};
                        let auto=if matches!(options,JsValue::Object(_)){let value=self.js.binding_get_property(&options,"autoIncrement")?;self.js.binding_to_boolean(&value)}else{false};
                        let transaction=self.storage_upgrade_transaction(connection)?;
                        let result={let mut state=self.state.borrow_mut();let StorageObject::Transaction{value:Some(tx),..}=&mut state.storage.as_mut().unwrap().objects[transaction] else{unreachable!()};
                            tx.database_mut().and_then(|db|db.create_store(name.clone(),key_path,auto))};
                        result.map_err(|error|self.storage_idb_error(error))?;
                        self.storage_allocate(SI::IDBObjectStore,StorageObject::Store{transaction,name,key_path_value:None})
                    },
                    "deleteObjectStore"=>{let name=self.storage_string(self.storage_arg(args,0)?)?;let transaction=self.storage_upgrade_transaction(connection)?;
                        let result={let mut state=self.state.borrow_mut();let StorageObject::Transaction{value:Some(tx),..}=&mut state.storage.as_mut().unwrap().objects[transaction] else{unreachable!()};
                            tx.database_mut().and_then(|db|db.delete_store(&name))};
                        result.map_err(|error|self.storage_idb_error(error))?;Ok(JsValue::Undefined)},
                    _=>Ok(JsValue::Null),
                }
            },
            SI::IDBTransaction=>self.storage_transaction_invoke(member,kind,this,args),
            SI::IDBObjectStore=>self.storage_store_invoke(member,this,args),
            SI::IDBIndex=>self.storage_index_invoke(member,this,args),
            SI::IDBCursor|SI::IDBCursorWithValue=>self.storage_cursor_invoke(interface,member,this,args),
            SI::IDBRecord=>{
                let slot=self.storage_slot(&this,interface)?;
                let (key,primary_key,value)={let state=self.state.borrow();
                    match &state.storage.as_ref().unwrap().objects[slot] {
                        StorageObject::Record{key,primary_key,value}=>(key.clone(),primary_key.clone(),value.clone()),
                        _=>unreachable!(),
                    }};
                match member {"key"=>Ok(key),"primaryKey"=>Ok(primary_key),"value"=>Ok(value),
                    _=>Err(Throw::type_error("Unknown IDBRecord member"))}
            },
            _=>Err(Throw::type_error("IndexedDB interface method unavailable")),
        }
    }
    fn storage_new_transaction(&mut self,connection:u64,names:Vec<String>,mode:Mode)->Result<JsValue,Throw>{
        let begun={let state=self.state.borrow();state.storage.as_ref().unwrap().idb.borrow_mut().begin(connection,names.clone(),mode,Durability::Default)};
        let (id,value,queued)=match begun {
            Ok((id,value))=>(id,Some(value),false),
            Err(idb::Error::Blocked)=>{
                let valid={let state=self.state.borrow();let storage=state.storage.as_ref().unwrap();
                    storage.idb.borrow().connection(connection).and_then(|c|storage.idb.borrow().database(&c.key,&c.name).map(|db|
                        names.iter().all(|name|db.store_names().contains(&name.as_str())))).unwrap_or(false)};
                if !valid {return Err(self.storage_dom_error("NotFoundError","Object store not found"))}
                (0,None,true)
            },
            Err(error)=>return Err(self.storage_idb_error(error)),
        };
        let wrapper=self.storage_allocate(SI::IDBTransaction,StorageObject::Transaction{connection,id,value,mode,scope:names,queued,active_for_requests:true,handlers:HashMap::new(),listeners:Vec::new()})?;
        let slot=self.storage_slot(&wrapper,SI::IDBTransaction)?;
        self.storage_schedule(StorageJob::TransactionComplete{transaction:slot})?;
        Ok(wrapper)
    }
    fn storage_upgrade_transaction(&mut self,connection:u64)->Result<usize,Throw>{
        let slot=self.state.borrow().storage.as_ref().unwrap().objects.iter().position(|object|
            matches!(object,StorageObject::Transaction{connection:id,mode:Mode::VersionChange,value:Some(_),..} if *id==connection));
        slot.ok_or_else(||self.storage_dom_error("InvalidStateError","No active upgrade transaction"))
    }
    fn storage_transaction_accepts_request(&mut self,slot:usize)->Result<(),Throw>{
        let active={let state=self.state.borrow();matches!(state.storage.as_ref().unwrap().objects.get(slot),
            Some(StorageObject::Transaction{value:Some(tx),active_for_requests:true,..}) if tx.is_active())};
        if active {Ok(())} else {Err(self.storage_dom_error("TransactionInactiveError","Transaction is inactive"))}
    }
    fn storage_transaction_invoke(&mut self,member:&str,kind:SM,this:JsValue,args:&[JsValue])->Result<JsValue,Throw>{
        let slot=self.storage_slot(&this,SI::IDBTransaction)?;
        if member.starts_with("on") {
            if kind==SM::Setter {let callback=args.first().cloned().unwrap_or(JsValue::Null);
                if let StorageObject::Transaction{handlers,..}=&mut self.state.borrow_mut().storage.as_mut().unwrap().objects[slot]{handlers.insert(member[2..].to_owned(),callback);}
                return Ok(JsValue::Undefined)}
            let state=self.state.borrow();if let StorageObject::Transaction{handlers,..}=&state.storage.as_ref().unwrap().objects[slot]{return Ok(handlers.get(&member[2..]).cloned().unwrap_or(JsValue::Null))}
        }
        let (connection,mode,names,active,live)={let state=self.state.borrow();let StorageObject::Transaction{connection,mode,scope,value,queued,active_for_requests,..}=&state.storage.as_ref().unwrap().objects[slot] else{unreachable!()};
            (*connection,*mode,scope.clone(),value.as_ref().is_some_and(IdbTransaction::is_active)||*queued,*active_for_requests)};
        match member {
            "mode"=>Ok(JsValue::str(match mode{Mode::ReadOnly=>"readonly",Mode::ReadWrite=>"readwrite",Mode::VersionChange=>"versionchange"})),
            "durability"=>Ok(JsValue::str("default")),"error"=>Ok(JsValue::Null),
            "objectStoreNames"=>self.js.binding_create_array(names.into_iter().map(JsValue::str).collect()),
            "db"=>{let state=self.state.borrow();Ok(state.storage.as_ref().unwrap().objects.iter().enumerate().find_map(|(index,object)|
                matches!(object,StorageObject::Database{connection:id,..} if *id==connection).then(||state.storage.as_ref().unwrap().wrappers.get(&index).cloned()).flatten()).unwrap_or(JsValue::Null))},
            "objectStore"=>{if !active{return Err(self.storage_dom_error("TransactionInactiveError","Transaction has finished"))}
                let name=self.storage_string(self.storage_arg(args,0)?)?;
                let found={let state=self.state.borrow();let storage=state.storage.as_ref().unwrap();
                    match &storage.objects[slot] {
                        StorageObject::Transaction{value:Some(tx),..}=>tx.store(&name).is_ok(),
                        StorageObject::Transaction{queued:true,..}=>{
                            let idb=storage.idb.borrow();
                            idb.connection(connection).and_then(|connection|idb.database(&connection.key,&connection.name))
                                .is_some_and(|db|db.store_names().contains(&name.as_str()))
                        },
                        _=>false,
                    }
                };
                if !found {return Err(self.storage_dom_error("NotFoundError","Object store not found"))}
                self.storage_allocate(SI::IDBObjectStore,StorageObject::Store{transaction:slot,name,key_path_value:None})},
            "abort"=>{if !active||!live{return Err(self.storage_dom_error("InvalidStateError","Transaction has finished or is committing"))}
                let mut state=self.state.borrow_mut();if let StorageObject::Transaction{value:Some(tx),..}=&mut state.storage.as_mut().unwrap().objects[slot]{tx.abort();}Ok(JsValue::Undefined)},
            "commit"=>Ok(JsValue::Undefined),
            _=>Err(Throw::type_error("Unknown IDBTransaction member")),
        }
    }
    fn storage_store_invoke(&mut self,member:&str,this:JsValue,args:&[JsValue])->Result<JsValue,Throw>{
        let slot=self.storage_slot(&this,SI::IDBObjectStore)?;
        let (transaction,name)={let state=self.state.borrow();match &state.storage.as_ref().unwrap().objects[slot]{StorageObject::Store{transaction,name,..}=>(*transaction,name.clone()),_=>unreachable!()}};
        match member {
            "name"=>return Ok(JsValue::str(name)),"transaction"=>return self.storage_wrapper(transaction),
            "keyPath"|"autoIncrement"|"indexNames"=>{
                let outcome={let state=self.state.borrow();let StorageObject::Transaction{value:Some(tx),..}=&state.storage.as_ref().unwrap().objects[transaction]else{return Err(Throw::type_error("Inactive transaction"))};
                    tx.store(&name).map(|store|(store.key_path.clone(),store.auto_increment,
                        store.index_names().into_iter().map(str::to_owned).collect::<Vec<_>>()))};
                // Reached only through a handle that named an existing store
                // when it was created, so a NotFound here means the store was
                // deleted since -- InvalidStateError, not NotFoundError.
                let (key_path,auto,names)=outcome.map_err(|error|self.storage_idb_error_for_handle(error))?;
                return match member {"keyPath"=>key_path.as_ref().map_or(Ok(JsValue::Null),|path|self.storage_cached_key_path_value(slot,path)),
                    "autoIncrement"=>Ok(JsValue::Bool(auto)),_=>self.js.binding_create_array(names.into_iter().map(JsValue::str).collect())};
            },
            _=>{},
        }
        if matches!(member,"createIndex"|"index"|"deleteIndex") {
            if member!="index" {self.storage_transaction_accepts_request(transaction)?;}
            let index_name=self.storage_string(self.storage_arg(args,0)?)?;
            if member=="index" {
                // `tx.store(&name)` failing means this IDBObjectStore handle's
                // own store was deleted (InvalidStateError); `store.index(...)`
                // failing is a fresh by-name lookup with no such index
                // (NotFoundError) -- tag which stage produced the error so each
                // gets the right DOMException.
                let outcome={let state=self.state.borrow();let StorageObject::Transaction{value:Some(tx),..}=&state.storage.as_ref().unwrap().objects[transaction] else{return Err(Throw::type_error("Inactive transaction"))};
                    tx.store(&name).map_err(|error|(true,error)).and_then(|store|store.index(&index_name).map(|_|()).map_err(|error|(false,error)))};
                if let Err((own_store,error))=outcome{return Err(if own_store{self.storage_idb_error_for_handle(error)}else{self.storage_idb_error(error)})}
                return self.storage_allocate(SI::IDBIndex,StorageObject::Index{transaction,store:name,name:index_name,key_path_value:None,object_store_value:None});
            }
            if member=="createIndex" {
                let path=self.storage_parse_key_path(self.storage_arg(args,1)?)?
                    .ok_or_else(||self.storage_dom_error("SyntaxError","Index key path is required"))?;
                let options=args.get(2).cloned().unwrap_or(JsValue::Undefined);
                let unique=if matches!(options,JsValue::Object(_)){let value=self.js.binding_get_property(&options,"unique")?;self.js.binding_to_boolean(&value)}else{false};
                let multi=if matches!(options,JsValue::Object(_)){let value=self.js.binding_get_property(&options,"multiEntry")?;self.js.binding_to_boolean(&value)}else{false};
                let outcome={let mut state=self.state.borrow_mut();let StorageObject::Transaction{value:Some(tx),mode:Mode::VersionChange,..}=&mut state.storage.as_mut().unwrap().objects[transaction] else{return Err(Throw::type_error("No active upgrade transaction"))};
                    tx.store_mut(&name).map_err(|error|(true,error)).and_then(|store|store.create_index(index_name.clone(),path,unique,multi).map_err(|error|(false,error)))};
                if let Err((own_store,error))=outcome{return Err(if own_store{self.storage_idb_error_for_handle(error)}else{self.storage_idb_error(error)})}
                return self.storage_allocate(SI::IDBIndex,StorageObject::Index{transaction,store:name,name:index_name,key_path_value:None,object_store_value:None});
            }
            let outcome={let mut state=self.state.borrow_mut();let StorageObject::Transaction{value:Some(tx),mode:Mode::VersionChange,..}=&mut state.storage.as_mut().unwrap().objects[transaction] else{return Err(Throw::type_error("No active upgrade transaction"))};
                tx.store_mut(&name).map_err(|error|(true,error)).and_then(|store|store.delete_index(&index_name).map_err(|error|(false,error)))};
            if let Err((own_store,error))=outcome{return Err(if own_store{self.storage_idb_error_for_handle(error)}else{self.storage_idb_error(error)})}
            return Ok(JsValue::Undefined);
        }
        if member=="openCursor"||member=="openKeyCursor" {
            self.storage_transaction_accepts_request(transaction)?;
            let range=self.storage_query_range(args.first())?;
            let direction=self.storage_direction(args.get(1))?;
            let outcome={let state=self.state.borrow();let StorageObject::Transaction{value:Some(tx),..}=&state.storage.as_ref().unwrap().objects[transaction] else{return Err(Throw::type_error("Inactive transaction"))};
                tx.store(&name).map(|store|store.cursor(range.as_ref(),direction))};
            let cursor=outcome.map_err(|error|self.storage_idb_error_for_handle(error))?;
            return self.storage_open_cursor(cursor,transaction,slot,this,member=="openCursor");
        }
        if matches!(member,"getAll"|"getAllKeys"|"getAllRecords") {
            self.storage_transaction_accepts_request(transaction)?;
            let (range,count,direction)=self.storage_get_all_args(args)?;
            let outcome={let state=self.state.borrow();let StorageObject::Transaction{value:Some(tx),..}=&state.storage.as_ref().unwrap().objects[transaction] else{return Err(Throw::type_error("Inactive transaction"))};
                tx.store(&name).map(|store|store.cursor(range.as_ref(),direction).all(count).to_vec())};
            let records=outcome.map_err(|error|self.storage_idb_error_for_handle(error))?;
            let (request,wrapper)=self.storage_request(false,this,self.storage_wrapper(transaction)?)?;
            let result=match member {
                "getAll"=>{let mut items=Vec::new();for record in &records{items.push(self.storage_from_value(&record.value)?)}self.js.binding_create_array(items)?},
                "getAllKeys"=>{let mut items=Vec::new();for record in &records{items.push(self.storage_from_key(&record.primary_key)?)}self.js.binding_create_array(items)?},
                _=>{let mut items=Vec::new();for record in &records{items.push(self.storage_create_record(&record.key,&record.primary_key,&record.value)?)}self.js.binding_create_array(items)?},
            };
            self.storage_schedule(StorageJob::RequestSuccess{request,result})?;
            return Ok(wrapper);
        }
        self.storage_transaction_accepts_request(transaction)?;
        let (key,range)=if matches!(member,"add"|"put") {
            let key=args.get(1).map(|value|self.storage_to_key(value)).transpose()?;(key,None)
        }else if let Some(value)=args.first(){
            if let Some(handle)=self.js.binding_host_handle(value,STORAGE_BRAND_BASE+SI::IDBKeyRange as u64){
                let state=self.state.borrow();let range=match &state.storage.as_ref().unwrap().objects[handle.slot as usize]{StorageObject::Range(range)=>range.clone(),_=>unreachable!()};(None,Some(range))
            }else{let key=self.storage_to_key(value)?;(Some(key.clone()),Some(KeyRange::only(key)))}
        }else{(None,None)};
        let value=if matches!(member,"add"|"put"){Some(self.storage_clone_for_idb(self.storage_arg(args,0)?)?)}else{None};
        let outcome={let mut state=self.state.borrow_mut();let storage=state.storage.as_mut().unwrap();
            let StorageObject::Transaction{value:Some(tx),..}=&mut storage.objects[transaction]else{return Err(Throw::type_error("Inactive transaction"))};
            match member {
                "add"|"put"=>tx.store_mut(&name).and_then(|store|store.put(value.unwrap(),key,member=="put")).map(StoreResult::Key),
                "get"|"getKey"|"count"=>tx.store(&name).map(|store|{
                    let records=store.records(range.as_ref(),None);
                    match member {"get"=>StoreResult::Value(records.first().map(|(_,value)|value.clone())),
                        "getKey"=>StoreResult::OptionalKey(records.first().map(|(key,_)|key.clone())),
                        _=>StoreResult::Count(store.count(range.as_ref()))}
                }),
                // A key range deletes every record it matches, not just a
                // single key -- store.delete() only takes one key, so a
                // range-form delete() (no `key`, only `range`) must walk the
                // matching keys itself.
                "delete"=>tx.store_mut(&name).map(|store|{
                    if let Some(key)=key.as_ref(){store.delete(key);}
                    else if let Some(range)=range.as_ref(){
                        for existing in store.records(Some(range),None).into_iter().map(|(key,_)|key).collect::<Vec<_>>(){store.delete(&existing);}
                    }
                    StoreResult::None
                }),
                "clear"=>tx.store_mut(&name).map(|store|{store.clear();StoreResult::None}),
                _=>Err(idb::Error::InvalidState),
            }
        };
        if let Err(error)=&outcome {
            if matches!(error,idb::Error::Data|idb::Error::ReadOnly|idb::Error::TransactionInactive|idb::Error::InvalidState) {
                return Err(self.storage_idb_error(error.clone()))
            }
            if matches!(error,idb::Error::NotFound) {
                return Err(self.storage_idb_error_for_handle(error.clone()))
            }
        }
        let (request,wrapper)=self.storage_request(false,this,self.storage_wrapper(transaction)?)?;
        let result=match outcome {
            Ok(StoreResult::None)=>JsValue::Undefined,Ok(StoreResult::Count(count))=>JsValue::num(count as f64),
            Ok(StoreResult::Key(key))=>self.storage_from_key(&key)?,
            Ok(StoreResult::OptionalKey(key))=>key.as_ref().map_or(Ok(JsValue::Undefined),|key|self.storage_from_key(key))?,
            Ok(StoreResult::Value(value))=>value.as_ref().map_or(Ok(JsValue::Undefined),|value|self.storage_from_value(value))?,
            Err(error)=>{let error=self.storage_idb_error(error).0;self.storage_schedule(StorageJob::RequestError{request,error})?;return Ok(wrapper)},
        };
        self.storage_schedule(StorageJob::RequestSuccess{request,result})?;
        Ok(wrapper)
    }
    fn storage_query_range(&mut self,value:Option<&JsValue>)->Result<Option<KeyRange>,Throw>{
        let Some(value)=value else{return Ok(None)};
        if matches!(value,JsValue::Undefined|JsValue::Null){return Ok(None)}
        if let Some(handle)=self.js.binding_host_handle(value,STORAGE_BRAND_BASE+SI::IDBKeyRange as u64){
            let state=self.state.borrow();return match &state.storage.as_ref().unwrap().objects[handle.slot as usize]{StorageObject::Range(range)=>Ok(Some(range.clone())),_=>Err(Throw::type_error("Invalid key range"))}
        }
        Ok(Some(KeyRange::only(self.storage_to_key(value)?)))
    }
    // getAll()/getAllKeys()/getAllRecords() share one argument shape: the
    // first argument is either the classic query (a key, an IDBKeyRange, or
    // omitted) or -- since getAllRecords() shipped alongside an IDBGetAllOptions
    // overload for all three methods -- a plain {query, count, direction}
    // dictionary. A bare key was never valid as that dictionary (an IDB key is
    // never a plain, non-Array/Date/binary object), so any plain object here is
    // unambiguously the options form; the legacy second `count` argument still
    // applies when the dictionary itself has no `count`.
    fn storage_get_all_args(&mut self,args:&[JsValue])->Result<(Option<KeyRange>,Option<usize>,Direction),Throw>{
        let mut range=None;let mut count=None;let mut direction=Direction::Next;
        if let Some(first)=args.first().filter(|v|!matches!(v,JsValue::Undefined|JsValue::Null)) {
            let treat_as_query=!matches!(first,JsValue::Object(_))
                ||self.js.binding_host_handle(first,STORAGE_BRAND_BASE+SI::IDBKeyRange as u64).is_some()
                ||matches!(first,JsValue::Object(object) if matches!(object.borrow().exotic,Exotic::Array))
                ||matches!(first,JsValue::Object(object) if matches!(object.borrow().get_own("[[DateValue]]"),Some(JsValue::Num(_))))
                ||self.storage_binary_bytes(first)?.is_some();
            if treat_as_query {
                range=self.storage_query_range(Some(first))?;
            } else {
                let query=self.js.binding_get_property(first,"query")?;
                range=self.storage_query_range(Some(&query))?;
                let count_value=self.js.binding_get_property(first,"count")?;
                if !matches!(count_value,JsValue::Undefined){count=Some(self.js.binding_to_number(&count_value)?.max(0.0) as usize)}
                let direction_value=self.js.binding_get_property(first,"direction")?;
                if !matches!(direction_value,JsValue::Undefined){direction=self.storage_direction(Some(&direction_value))?;}
            }
        }
        if count.is_none() {
            count=args.get(1).filter(|v|!matches!(v,JsValue::Undefined)).map(|v|self.js.binding_to_number(v).map(|n|n.max(0.0) as usize)).transpose()?;
        }
        // Per spec, a count of 0 (explicit or omitted) means "no limit", not "zero results".
        if count==Some(0) {count=None}
        Ok((range,count,direction))
    }
    fn storage_create_record(&mut self,key:&Key,primary_key:&Key,value:&StructuredValue)->Result<JsValue,Throw>{
        let key_value=self.storage_from_key(key)?;
        let primary_key_value=self.storage_from_key(primary_key)?;
        let value_value=self.storage_from_value(value)?;
        self.storage_allocate(SI::IDBRecord,StorageObject::Record{key:key_value,primary_key:primary_key_value,value:value_value})
    }
    fn storage_parse_key_path(&mut self,value:&JsValue)->Result<Option<idb::KeyPath>,Throw>{
        if matches!(value,JsValue::Undefined|JsValue::Null){return Ok(None)}
        let path=if let JsValue::Object(object)=value {
            if matches!(object.borrow().exotic,Exotic::Array){
                let length=self.js.binding_get_property(value,"length")?;
                let count=self.js.binding_to_number(&length)? as usize;
                if count>100_000{return Err(self.storage_dom_error("SyntaxError","Key path is too long"))}
                let mut parts=Vec::with_capacity(count);
                for index in 0..count {
                    let item=self.js.binding_get_property(value,&index.to_string())?;
                    parts.push(self.storage_string(&item)?);
                }
                idb::KeyPath::Compound(parts)
            }else{idb::KeyPath::Single(self.storage_string(value)?)}
        }else{idb::KeyPath::Single(self.storage_string(value)?)};
        let valid=match &path {idb::KeyPath::Single(path)=>Self::storage_valid_key_path(path),
            idb::KeyPath::Compound(paths)=>paths.iter().all(|path|Self::storage_valid_key_path(path))};
        if !valid{return Err(self.storage_dom_error("SyntaxError","Invalid key path"))}
        Ok(Some(path))
    }
    fn storage_valid_key_path(path:&str)->bool {
        if path.is_empty(){return true}
        path.split('.').all(|part|{
            let mut chars=part.chars();
            chars.next().is_some_and(|c|c=='_'||c=='$'||c.is_alphabetic())
                &&chars.all(|c|c=='_'||c=='$'||c=='\u{200c}'||c=='\u{200d}'||c.is_alphanumeric())
        })
    }
    fn storage_key_path_value(&mut self,path:&idb::KeyPath)->Result<JsValue,Throw>{
        match path {idb::KeyPath::Single(path)=>Ok(JsValue::str(path)),
            idb::KeyPath::Compound(paths)=>self.js.binding_create_array(paths.iter().map(JsValue::str).collect())}
    }
    fn storage_cached_key_path_value(&mut self,slot:usize,path:&idb::KeyPath)->Result<JsValue,Throw>{
        let cached={let state=self.state.borrow();match &state.storage.as_ref().unwrap().objects[slot]{
            StorageObject::Store{key_path_value,..}|StorageObject::Index{key_path_value,..}=>key_path_value.clone(),
            _=>None}};
        if let Some(value)=cached{return Ok(value)}
        let value=self.storage_key_path_value(path)?;
        let mut state=self.state.borrow_mut();match &mut state.storage.as_mut().unwrap().objects[slot]{
            StorageObject::Store{key_path_value,..}|StorageObject::Index{key_path_value,..}=>*key_path_value=Some(value.clone()),
            _=>{},
        }
        Ok(value)
    }
    fn storage_direction(&mut self,value:Option<&JsValue>)->Result<Direction,Throw>{
        let Some(value)=value else{return Ok(Direction::Next)};
        match self.storage_string(value)?.as_str(){"next"=>Ok(Direction::Next),"nextunique"=>Ok(Direction::NextUnique),
            "prev"=>Ok(Direction::Prev),"prevunique"=>Ok(Direction::PrevUnique),_=>Err(Throw::type_error("Invalid cursor direction"))}
    }
    fn storage_open_cursor(&mut self,mut cursor:IdbCursor,transaction:usize,source:usize,source_value:JsValue,with_value:bool)->Result<JsValue,Throw>{
        let present=cursor.advance(1).map_err(|error|self.storage_idb_error(error))?.is_some();
        let (request,wrapper)=self.storage_request(false,source_value,self.storage_wrapper(transaction)?)?;
        let result=if present {let kind=if with_value{SI::IDBCursorWithValue}else{SI::IDBCursor};
            self.storage_allocate(kind,StorageObject::Cursor{cursor,transaction,source,request,with_value})?}else{JsValue::Null};
        self.storage_schedule(StorageJob::RequestSuccess{request,result})?;
        Ok(wrapper)
    }
    fn storage_index_invoke(&mut self,member:&str,this:JsValue,args:&[JsValue])->Result<JsValue,Throw>{
        let slot=self.storage_slot(&this,SI::IDBIndex)?;
        let (transaction,store_name,index_name)={let state=self.state.borrow();match &state.storage.as_ref().unwrap().objects[slot]{
            StorageObject::Index{transaction,store,name,..}=>(*transaction,store.clone(),name.clone()),_=>unreachable!()}};
        if member=="name" {return Ok(JsValue::str(index_name))}
        if member=="objectStore" {
            if let StorageObject::Index{object_store_value:Some(value),..}=&self.state.borrow().storage.as_ref().unwrap().objects[slot]{return Ok(value.clone())}
            let value=self.storage_allocate(SI::IDBObjectStore,StorageObject::Store{transaction,name:store_name.clone(),key_path_value:None})?;
            if let StorageObject::Index{object_store_value,..}=&mut self.state.borrow_mut().storage.as_mut().unwrap().objects[slot]{*object_store_value=Some(value.clone());}
            return Ok(value)
        }
        if matches!(member,"keyPath"|"unique"|"multiEntry") {
            // This handle already named an existing store+index when it was
            // created, so any NotFound now -- from either lookup -- means
            // one of them was deleted since: InvalidStateError throughout.
            let outcome={let state=self.state.borrow();let StorageObject::Transaction{value:Some(tx),..}=&state.storage.as_ref().unwrap().objects[transaction]else{return Err(Throw::type_error("Inactive transaction"))};
                tx.store(&store_name).and_then(|store|store.index(&index_name))
                    .map(|index|(index.key_path.clone(),index.unique,index.multi_entry))};
            let (path,unique,multi)=outcome.map_err(|error|self.storage_idb_error_for_handle(error))?;
            return match member {"keyPath"=>self.storage_cached_key_path_value(slot,&path),
                "unique"=>Ok(JsValue::Bool(unique)),_=>Ok(JsValue::Bool(multi))}
        }
        if member=="openCursor"||member=="openKeyCursor" {
            let range=self.storage_query_range(args.first())?;
            self.storage_transaction_accepts_request(transaction)?;
            let direction=self.storage_direction(args.get(1))?;
            let outcome={let state=self.state.borrow();let StorageObject::Transaction{value:Some(tx),..}=&state.storage.as_ref().unwrap().objects[transaction]else{return Err(Throw::type_error("Inactive transaction"))};
                tx.store(&store_name).and_then(|store|store.index(&index_name).map(|index|index.cursor(store,range.as_ref(),direction)))};
            let cursor=outcome.map_err(|error|self.storage_idb_error_for_handle(error))?;
            return self.storage_open_cursor(cursor,transaction,slot,this,member=="openCursor");
        }
        if matches!(member,"getAll"|"getAllKeys"|"getAllRecords") {
            let (range,count,direction)=self.storage_get_all_args(args)?;
            self.storage_transaction_accepts_request(transaction)?;
            let outcome={let state=self.state.borrow();let StorageObject::Transaction{value:Some(tx),..}=&state.storage.as_ref().unwrap().objects[transaction]else{return Err(Throw::type_error("Inactive transaction"))};
                tx.store(&store_name).and_then(|store|store.index(&index_name).map(|index|index.cursor(store,range.as_ref(),direction).all(count).to_vec()))};
            let records=outcome.map_err(|error|self.storage_idb_error_for_handle(error))?;
            let (request,wrapper)=self.storage_request(false,this,self.storage_wrapper(transaction)?)?;
            let result=match member {
                "getAll"=>{let mut items=Vec::new();for record in &records{items.push(self.storage_from_value(&record.value)?)}self.js.binding_create_array(items)?},
                "getAllKeys"=>{let mut items=Vec::new();for record in &records{items.push(self.storage_from_key(&record.primary_key)?)}self.js.binding_create_array(items)?},
                _=>{let mut items=Vec::new();for record in &records{items.push(self.storage_create_record(&record.key,&record.primary_key,&record.value)?)}self.js.binding_create_array(items)?},
            };
            self.storage_schedule(StorageJob::RequestSuccess{request,result})?;
            return Ok(wrapper);
        }
        if !matches!(member,"get"|"getKey"|"count"){return Err(Throw::type_error("Unknown IDBIndex member"))}
        let range=self.storage_query_range(args.first())?;
        self.storage_transaction_accepts_request(transaction)?;
        let outcome={let state=self.state.borrow();let StorageObject::Transaction{value:Some(tx),..}=&state.storage.as_ref().unwrap().objects[transaction]else{return Err(Throw::type_error("Inactive transaction"))};
            tx.store(&store_name).and_then(|store|store.index(&index_name).map(|index|{
                let records=index.records(store,range.as_ref(),None);
                match member {"get"=>StoreResult::Value(records.first().map(|(_,_,value)|value.clone())),
                    "getKey"=>StoreResult::OptionalKey(records.first().map(|(_,key,_)|key.clone())),
                    _=>StoreResult::Count(index.count(range.as_ref()))}
            }))};
        let outcome=outcome.map_err(|error|self.storage_idb_error_for_handle(error))?;
        let (request,wrapper)=self.storage_request(false,this,self.storage_wrapper(transaction)?)?;
        let result=match outcome {StoreResult::Value(value)=>value.as_ref().map_or(Ok(JsValue::Undefined),|value|self.storage_from_value(value))?,
            StoreResult::OptionalKey(key)=>key.as_ref().map_or(Ok(JsValue::Undefined),|key|self.storage_from_key(key))?,
            StoreResult::Count(count)=>JsValue::num(count as f64),_=>JsValue::Undefined};
        self.storage_schedule(StorageJob::RequestSuccess{request,result})?;Ok(wrapper)
    }
    fn storage_cursor_invoke(&mut self,interface:SI,member:&str,this:JsValue,args:&[JsValue])->Result<JsValue,Throw>{
        let slot=self.storage_slot(&this,interface)?;
        let (record,direction,source,request,transaction,with_value)={let state=self.state.borrow();let StorageObject::Cursor{cursor,source,request,transaction,with_value}= &state.storage.as_ref().unwrap().objects[slot]else{unreachable!()};
            (cursor.current().cloned(),cursor.direction(),*source,*request,*transaction,*with_value)};
        match member {
            "source"=>self.storage_wrapper(source),"request"=>self.storage_wrapper(request),
            "direction"=>Ok(JsValue::str(match direction{Direction::Next=>"next",Direction::NextUnique=>"nextunique",Direction::Prev=>"prev",Direction::PrevUnique=>"prevunique"})),
            "key"=>record.as_ref().map_or(Ok(JsValue::Undefined),|record|self.storage_from_key(&record.key)),
            "primaryKey"=>record.as_ref().map_or(Ok(JsValue::Undefined),|record|self.storage_from_key(&record.primary_key)),
            "value" if with_value=>record.as_ref().map_or(Ok(JsValue::Undefined),|record|self.storage_from_value(&record.value)),
            "advance"|"continue"|"continuePrimaryKey"=>{
                self.storage_transaction_accepts_request(transaction)?;
                let key=if member=="continue"&&args.first().is_some(){Some(self.storage_to_key(&args[0])?)}else{None};
                let pair=if member=="continuePrimaryKey"{Some((self.storage_to_key(self.storage_arg(args,0)?)?,self.storage_to_key(self.storage_arg(args,1)?)?))}else{None};
                let count=if member=="advance"{self.js.binding_to_number(self.storage_arg(args,0)?)? as usize}else{1};
                let result={let mut state=self.state.borrow_mut();let StorageObject::Cursor{cursor,..}=&mut state.storage.as_mut().unwrap().objects[slot]else{unreachable!()};
                    match member {"advance"=>cursor.advance(count),"continue"=>cursor.continue_to(key.as_ref()),
                        _=>{let (key,primary)=pair.as_ref().unwrap();cursor.continue_primary_key(key,primary)}}.map(|record|record.is_some())};
                let present=result.map_err(|error|self.storage_idb_error(error))?;
                if let Some(StorageObject::Request{ready,..})=self.state.borrow_mut().storage.as_mut().unwrap().objects.get_mut(request){*ready=false;}
                self.storage_schedule(StorageJob::RequestSuccess{request,result:if present{this}else{JsValue::Null}})?;
                Ok(JsValue::Undefined)
            },
            "update"|"delete"=>{
                self.storage_transaction_accepts_request(transaction)?;
                let record=record.ok_or_else(||self.storage_dom_error("InvalidStateError","Cursor exhausted"))?;
                let value=if member=="update"{Some(self.storage_clone_for_idb(self.storage_arg(args,0)?)?)}else{None};
                let (store_name,_)={let state=self.state.borrow();match &state.storage.as_ref().unwrap().objects[source]{
                    StorageObject::Store{name,..}=>(name.clone(),false),StorageObject::Index{store,..}=>(store.clone(),true),_=>return Err(Throw::type_error("Invalid cursor source"))}};
                let outcome={let mut state=self.state.borrow_mut();let StorageObject::Transaction{value:Some(tx),..}=&mut state.storage.as_mut().unwrap().objects[transaction]else{return Err(Throw::type_error("Inactive transaction"))};
                    tx.store_mut(&store_name).and_then(|store|if let Some(value)=value{store.put(value,Some(record.primary_key.clone()),true).map(StoreResult::Key)}else{store.delete(&record.primary_key);Ok(StoreResult::None)})};
                let (request,wrapper)=self.storage_request(false,this,self.storage_wrapper(transaction)?)?;
                match outcome {Ok(StoreResult::Key(key))=>{let result=self.storage_from_key(&key)?;self.storage_schedule(StorageJob::RequestSuccess{request,result})?},
                    Ok(_)=>self.storage_schedule(StorageJob::RequestSuccess{request,result:JsValue::Undefined})?,
                    Err(error)=>{let error=self.storage_idb_error(error).0;self.storage_schedule(StorageJob::RequestError{request,error})?;}};
                Ok(wrapper)
            },
            _=>Err(Throw::type_error("Unknown IDBCursor member")),
        }
    }
    pub(super) fn storage_tick(&mut self) -> Result<JsValue, Throw> {
        let job={let mut state=self.state.borrow_mut();let storage=state.storage.as_mut().unwrap();storage.tick_scheduled=false;storage.pending.pop_front()};
        if let Some(job)=job {
            match job {
                StorageJob::LockGranted{id}=>self.storage_lock_granted(id)?,
                StorageJob::RequestSuccess{request,result}=>{
                    if let Some(StorageObject::Request{result:stored,ready,error,..})=self.state.borrow_mut().storage.as_mut().unwrap().objects.get_mut(request){
                        *stored=result;*ready=true;*error=JsValue::Null;
                    }
                    self.storage_activate_request_transaction(request);
                    self.storage_dispatch(request,"success",None,None)?;
                },
                StorageJob::RequestError{request,error}=>{
                    if let Some(StorageObject::Request{error:stored,ready,..})=self.state.borrow_mut().storage.as_mut().unwrap().objects.get_mut(request){
                        *stored=error;*ready=true;
                    }
                    self.storage_activate_request_transaction(request);
                    self.storage_dispatch(request,"error",None,None)?;
                    if let Some(StorageObject::Request{source:JsValue::Null,transaction,..})=
                        self.state.borrow_mut().storage.as_mut().unwrap().objects.get_mut(request){*transaction=JsValue::Null;}
                },
                StorageJob::DeleteSuccess{request,old}=>{
                    if let Some(StorageObject::Request{result,error,ready,..})=self.state.borrow_mut().storage.as_mut().unwrap().objects.get_mut(request){
                        *result=JsValue::Undefined;*error=JsValue::Null;*ready=true;
                    }
                    self.storage_dispatch(request,"success",Some(old),None)?;
                },
                StorageJob::OpenBlocked{request,old,new}=>{
                    let still_blocked={let state=self.state.borrow();let storage=state.storage.as_ref().unwrap();
                        storage.blocked_opens.iter().any(|(_,_,slot)|*slot==request)
                            ||storage.blocked_deletes.iter().any(|(_,_,slot)|*slot==request)};
                    if still_blocked {self.storage_dispatch(request,"blocked",Some(old),new)?;}
                },
                StorageJob::DatabaseVersionChange{connection,old,new}=>{
                    let slots={let state=self.state.borrow();state.storage.as_ref().unwrap().objects.iter().enumerate()
                        .filter_map(|(slot,object)|matches!(object,StorageObject::Database{connection:id,..} if *id==connection).then_some(slot)).collect::<Vec<_>>()};
                    for slot in slots {self.storage_dispatch(slot,"versionchange",Some(old),new)?;}
                },
                StorageJob::DispatchListener{slot,event,mut callbacks}=>{
                    self.storage_activate_request_transaction(slot);
                    if let Some(callback)=callbacks.pop_front(){
                        let target=self.storage_wrapper(slot)?;
                        binding_call(self.js,&callback,&target,&[event.clone()])?;
                    }
                    if !callbacks.is_empty(){self.state.borrow_mut().storage.as_mut().unwrap().pending.push_front(
                        StorageJob::DispatchListener{slot,event,callbacks});}
                },
                StorageJob::OpenUpgrade{request,connection,name,key,old,new}=>{
                    let database=self.storage_allocate(SI::IDBDatabase,StorageObject::Database{connection,name,key,handlers:HashMap::new(),listeners:Vec::new()})?;
                    let (txid,tx)={let state=self.state.borrow();state.storage.as_ref().unwrap().idb.borrow_mut().begin(connection,Vec::<String>::new(),Mode::VersionChange,Durability::Default)}
                        .map_err(|error|self.storage_idb_error(error))?;
                    let transaction=self.storage_allocate(SI::IDBTransaction,StorageObject::Transaction{connection,id:txid,value:Some(tx),mode:Mode::VersionChange,scope:Vec::new(),queued:false,active_for_requests:true,handlers:HashMap::new(),listeners:Vec::new()})?;
                    let transaction_slot=self.storage_slot(&transaction,SI::IDBTransaction)?;
                    if let Some(StorageObject::Request{result,transaction:stored,ready,error,..})=self.state.borrow_mut().storage.as_mut().unwrap().objects.get_mut(request){
                        *result=database.clone();*stored=transaction;*ready=true;*error=JsValue::Null;
                    }
                    self.storage_dispatch(request,"upgradeneeded",Some(old),Some(new))?;
                    self.storage_schedule(StorageJob::TransactionComplete{transaction:transaction_slot})?;
                    // Do NOT eagerly schedule the open request's "success" here: per spec,
                    // it must not fire until this versionchange transaction has actually
                    // auto-committed. TransactionComplete defers itself (see `waiting`
                    // below) whenever a request placed against the transaction (e.g. an
                    // `add()` inside `onupgradeneeded`) hasn't been delivered yet, which
                    // reschedules it to the *back* of the queue -- behind an already-queued
                    // RequestSuccess for this same open request, so "success" fired before
                    // the store the upgrade created was actually committed to the database,
                    // and every later `db.transaction(...)` in that success handler saw a
                    // pre-upgrade schema ("Object store not found"). TransactionComplete's
                    // commit branch schedules RequestSuccess itself once it truly commits.
                },
                StorageJob::TransactionComplete{transaction}=>{
                    let queued={let state=self.state.borrow();match state.storage.as_ref().unwrap().objects.get(transaction){
                        Some(StorageObject::Transaction{connection,mode,scope,queued:true,..})=>Some((*connection,*mode,scope.clone())),_=>None}};
                    if let Some((connection,mode,scope))=queued {
                        let begun={let state=self.state.borrow();state.storage.as_ref().unwrap().idb.borrow_mut().begin(connection,scope,mode,Durability::Default)};
                        match begun {
                            Ok((id,value))=>{let mut state=self.state.borrow_mut();if let StorageObject::Transaction{id:stored,value:stored_value,queued,..}=
                                &mut state.storage.as_mut().unwrap().objects[transaction]{*stored=id;*stored_value=Some(value);*queued=false;}},
                            Err(idb::Error::Blocked)=>{self.storage_schedule(StorageJob::TransactionComplete{transaction})?;return Ok(JsValue::Undefined)},
                            Err(error)=>return Err(self.storage_idb_error(error)),
                        }
                    }
                    let waiting={let state=self.state.borrow();let storage=state.storage.as_ref().unwrap();let wrapper=storage.wrappers.get(&transaction);
                        storage.pending.iter().any(|job|match job {StorageJob::RequestSuccess{request,..}|StorageJob::RequestError{request,..}=>{
                            matches!(storage.objects.get(*request),Some(StorageObject::Request{transaction:value,source,..}) if !matches!(source,JsValue::Null) && wrapper.is_some_and(|wrapper|self.js.binding_same_value(wrapper,value)))},_=>false})};
                    if waiting {self.storage_schedule(StorageJob::TransactionComplete{transaction})?;
                        return Ok(JsValue::Undefined)}
                    let (connection,id,value)={let mut state=self.state.borrow_mut();
                        match state.storage.as_mut().unwrap().objects.get_mut(transaction){Some(StorageObject::Transaction{connection,id,value,..})=>(*connection,*id,value.take()),_=>return Ok(JsValue::Undefined)}};
                    if let Some(value)=value {
                        // The open request (if any) whose `.transaction` is this versionchange
                        // transaction: its "success"/"error" event must fire only now, after
                        // the transaction has actually committed or aborted -- not eagerly when
                        // the upgrade started (see OpenUpgrade above).
                        let open_request={let state=self.state.borrow();let storage=state.storage.as_ref().unwrap();
                            let wrapper=storage.wrappers.get(&transaction);
                            storage.objects.iter().enumerate().find_map(|(index,object)|match object{
                                StorageObject::Request{source:JsValue::Null,transaction:value,..}
                                    if wrapper.is_some_and(|wrapper|self.js.binding_same_value(wrapper,value))=>Some(index),
                                _=>None,
                            })};
                        let result=self.state.borrow().storage.as_ref().unwrap().idb.borrow_mut().finish(connection,id,value,true);
                        match result {
                            Ok(())=>{
                                self.storage_dispatch(transaction,"complete",None,None)?;
                                if let Some(request)=open_request {
                                    let database={let state=self.state.borrow();match state.storage.as_ref().unwrap().objects.get(request){
                                        Some(StorageObject::Request{result,..})=>result.clone(),_=>JsValue::Undefined}};
                                    self.storage_schedule(StorageJob::RequestSuccess{request,result:database})?;
                                }
                            },
                            Err(error)=>{
                                self.storage_dispatch(transaction,"abort",None,None)?;
                                if let Some(request)=open_request {
                                    self.state.borrow_mut().storage.as_mut().unwrap().pending.retain(|job|
                                        !matches!(job,StorageJob::RequestSuccess{request:slot,..} if *slot==request));
                                    let error=self.storage_idb_error(error).0;
                                    self.storage_schedule(StorageJob::RequestError{request,error})?;
                                }
                            },
                        }
                    }
                },
                StorageJob::StorageEvent(delivery)=>self.dispatch_storage_event(delivery)?,
            }
        }
        let more={let state=self.state.borrow();let storage=state.storage.as_ref().unwrap();!storage.pending.is_empty()&&!storage.tick_scheduled};
        if more {
            let callback=self.native("storage task",0,NativeEntry::StorageTick,HostCallPolicy::CallOnly);
            self.state.borrow_mut().storage.as_mut().unwrap().tick_scheduled=true;
            self.js.schedule_timer(callback,0,Vec::new())?;
        }
        Ok(JsValue::Undefined)
    }
    fn storage_activate_request_transaction(&mut self,request:usize){
        let value={let state=self.state.borrow();match state.storage.as_ref().unwrap().objects.get(request){
            Some(StorageObject::Request{transaction,..})=>transaction.clone(),_=>return}};
        let Ok(slot)=self.storage_slot(&value,SI::IDBTransaction) else{return};
        if let Some(StorageObject::Transaction{value:Some(_),active_for_requests,..})=
            self.state.borrow_mut().storage.as_mut().unwrap().objects.get_mut(slot){*active_for_requests=true;}
    }
    fn storage_dispatch(&mut self,slot:usize,name:&str,old:Option<u64>,new:Option<u64>)->Result<(),Throw>{
        let (handler,listeners)={let state=self.state.borrow();let object=&state.storage.as_ref().unwrap().objects[slot];
            let (handlers,listeners)=match object {
                StorageObject::Request{handlers,listeners,..}|StorageObject::Transaction{handlers,listeners,..}|StorageObject::Database{handlers,listeners,..}=>(handlers,listeners),
                _=>return Ok(()),
            };
            (handlers.get(name).cloned(),listeners.iter().filter(|(kind,_)|kind==name).map(|(_,callback)|callback.clone()).collect::<Vec<_>>())
        };
        let event=if let Some(old)=old {self.storage_version_event(name,old,new)?} else {let event=self.js.binding_create_object(None)?;
            self.define_data_property(&event,"type",JsValue::str(name),true,true,true)?;event};
        let target=self.storage_wrapper(slot)?;
        self.define_data_property(&event,"target",target.clone(),true,true,true)?;
        let mut callbacks=handler.into_iter().chain(listeners).filter(|callback|self.js.binding_is_callable(callback)).collect::<VecDeque<_>>();
        if let Some(callback)=callbacks.pop_front(){
            binding_call(self.js,&callback,&target,&[event.clone()])?;
        }
        if !callbacks.is_empty(){
            self.state.borrow_mut().storage.as_mut().unwrap().pending.push_front(
                StorageJob::DispatchListener{slot,event,callbacks});
        }
        Ok(())
    }
    fn storage_version_event(&mut self,name:&str,old:u64,new:Option<u64>)->Result<JsValue,Throw>{
        let prototype=self.state.borrow().storage_classes.get(&SI::IDBVersionChangeEvent)
            .ok_or_else(||Throw::type_error("IDBVersionChangeEvent is not installed"))?.0.clone();
        let event=self.js.binding_create_object(Some(&prototype))?;
        self.define_data_property(&event,"type",JsValue::str(name),true,true,true)?;
        self.define_data_property(&event,"oldVersion",JsValue::num(old as f64),true,true,true)?;
        self.define_data_property(&event,"newVersion",new.map_or(JsValue::Null,|version|JsValue::num(version as f64)),true,true,true)?;
        Ok(event)
    }
    fn storage_to_key(&mut self,value:&JsValue)->Result<Key,Throw>{
        let mut seen=Vec::new();
        self.storage_to_key_seen(value,&mut seen)
    }
    // `seen` tracks the array objects currently being converted (the ancestor
    // chain, not every array ever visited), so a self-referential array (`let
    // a=[];a.push(a)`, as IndexedDB/idbkeyrange-includes.any.js and
    // idbobjectstore_openKeyCursor.any.js construct) throws a DataError per the
    // "convert a value to a key" algorithm's cycle check, instead of recursing
    // until the native stack overflows and the process aborts (SIGABRT, not a
    // catchable Throw). Sibling/shared references at the same nesting level
    // (not an actual cycle) are unaffected: the entry is popped once this
    // array's own conversion finishes.
    fn storage_to_key_seen(&mut self,value:&JsValue,seen:&mut Vec<JsValue>)->Result<Key,Throw>{
        match value {
            JsValue::Num(number)=>Key::number(*number).map_err(|error|self.storage_idb_error(error)),
            JsValue::Str(string)=>Ok(Key::String(string.to_utf8_lossy())),
            JsValue::Object(object) if matches!(object.borrow().get_own("[[DateValue]]"),Some(JsValue::Num(_)))=>{
                let Some(JsValue::Num(value))=object.borrow().get_own("[[DateValue]]") else{unreachable!()};
                Key::date(value).map_err(|error|self.storage_idb_error(error))
            },
            JsValue::Object(object) if matches!(object.borrow().exotic,Exotic::Array)=>{
                if seen.iter().any(|ancestor|ancestor==value){
                    return Err(self.storage_dom_error("DataError","Invalid IndexedDB key: cyclic array"));
                }
                seen.push(value.clone());
                let length=self.js.binding_get_property(value,"length")?;
                let count=self.js.binding_to_number(&length)? as usize;
                if count>100_000{seen.pop();return Err(self.storage_dom_error("DataError","Key is too long"))}
                // Key conversion reads own elements only: a hole contributes no
                // key, and an inherited accessor at that index must not run.
                let mut items=Vec::new();
                for index in 0..count {
                    let name=index.to_string();
                    let has=match self.js.binding_get_own_property(value,&name){Ok(has)=>has,Err(error)=>{seen.pop();return Err(error)}};
                    if has.is_none(){continue}
                    let item=match self.js.binding_get_property(value,&name){Ok(item)=>item,Err(error)=>{seen.pop();return Err(error)}};
                    match self.storage_to_key_seen(&item,seen){Ok(key)=>items.push(key),Err(error)=>{seen.pop();return Err(error)}}
                }
                seen.pop();
                Ok(Key::Array(items))
            },
            JsValue::Object(_) if self.storage_binary_bytes(value)?.is_some()=>Ok(Key::Binary(self.storage_binary_bytes(value)?.unwrap())),
            _=>Err(self.storage_dom_error("DataError","Invalid IndexedDB key")),
        }
    }
    fn storage_binary_bytes(&mut self,value:&JsValue)->Result<Option<Vec<u8>>,Throw>{
        let JsValue::Object(_)=value else{return Ok(None)};
        let constructor=self.js.binding_get_property(value,"constructor")?;
        let name=self.js.binding_get_property(&constructor,"name")?;
        let JsValue::Str(name)=name else{return Ok(None)};
        let name=name.to_utf8_lossy();
        if name!="ArrayBuffer"&&!name.ends_with("Array")&&name!="DataView"{return Ok(None)}
        let input=if name=="ArrayBuffer"{value.clone()}else{self.js.binding_get_property(value,"buffer")?};
        let offset=if name=="ArrayBuffer"{0}else{let value=self.js.binding_get_property(value,"byteOffset")?;self.js.binding_to_number(&value)? as usize};
        let length=self.js.binding_get_property(value,"byteLength")?;
        let length=self.js.binding_to_number(&length)? as usize;
        if length>10_000_000{return Err(self.storage_dom_error("DataError","Binary key is too long"))}
        let global=self.js.binding_global_object();let constructor=self.js.binding_get_property(&global,"Uint8Array")?;
        let view=self.js.binding_construct(&constructor,vec![input,JsValue::num(offset as f64),JsValue::num(length as f64)])?;
        let mut bytes=Vec::with_capacity(length);
        for index in 0..length {let item=self.js.binding_get_property(&view,&index.to_string())?;bytes.push(self.js.binding_to_number(&item)? as u8)}
        Ok(Some(bytes))
    }
    fn storage_from_key(&mut self,key:&Key)->Result<JsValue,Throw>{
        match key {Key::Number(value)|Key::Date(value)=>Ok(JsValue::num(*value)),
            Key::String(value)=>Ok(JsValue::str(value)),Key::Binary(value)=>self.js.binding_create_array_buffer(value.clone()),
            Key::Array(items)=>{let mut values=Vec::new();for item in items {values.push(self.storage_from_key(item)?)}self.js.binding_create_array(values)}}
    }
    fn storage_clone_for_idb(&mut self,value:&JsValue)->Result<StructuredValue,Throw>{
        let mut host=StorageCloneHost{state:self.state.clone()};
        let wire=self.js.structured_serialize_with_host(value,&[],true,&mut host)?;
        Ok(StructuredValue::Graph{view:Box::new(idb::graph_view(&wire)),wire})
    }
    fn storage_from_value(&mut self,value:&StructuredValue)->Result<JsValue,Throw>{
        match value {StructuredValue::Graph{wire,..}=>{
            let mut host=StorageCloneHost{state:self.state.clone()};self.js.structured_deserialize_with_host(wire,&mut host)
        },
            StructuredValue::Hole|StructuredValue::Undefined=>Ok(JsValue::Undefined),StructuredValue::Null=>Ok(JsValue::Null),StructuredValue::Bool(value)=>Ok(JsValue::Bool(*value)),
            StructuredValue::Number(value)=>Ok(JsValue::num(*value)),
            StructuredValue::Date(value)=>{let global=self.js.binding_global_object();let constructor=self.js.binding_get_property(&global,"Date")?;self.js.binding_construct(&constructor,vec![JsValue::num(*value)])},
            StructuredValue::String(value)=>Ok(JsValue::str(value)),StructuredValue::Binary(value)=>self.js.binding_create_array_buffer(value.clone()),
            StructuredValue::Array(items)=>{let mut values=Vec::new();for item in items{values.push(self.storage_from_value(item)?)}
                let array=self.js.binding_create_array(values)?;
                for (index,item) in items.iter().enumerate(){if matches!(item,StructuredValue::Hole){self.js.binding_delete_property(&array,&index.to_string())?;}}
                Ok(array)},
            StructuredValue::Object(items)=>{let object=self.js.binding_create_object(None)?;
                for (name,value) in items {let item=self.storage_from_value(value)?;self.js.binding_set_property(&object,name,item)?;}Ok(object)},
        }
    }
}

#[cfg(test)]
mod storage_binding_tests {
    use std::{cell::RefCell, rc::Rc, time::Duration};
    use amazing_js::value::JsValue;
    use crate::{dom_host::BrowserDomHost, js_scheduler::DriverScheduler};
    use super::{TestStorageRoot,TEST_STORAGE_OVERRIDE};

    struct SharedTestStorage { _root: Rc<TestStorageRoot> }
    impl SharedTestStorage {
        fn new() -> Self {
            let root=TestStorageRoot::new();
            TEST_STORAGE_OVERRIDE.with(|slot|*slot.borrow_mut()=Some(root.clone()));
            Self{_root:root}
        }
    }
    impl Drop for SharedTestStorage {
        fn drop(&mut self){TEST_STORAGE_OVERRIDE.with(|slot|*slot.borrow_mut()=None);}
    }

    fn scheduler() -> DriverScheduler {
        let host=Rc::new(RefCell::new(BrowserDomHost{dom:amazing_html::parse("<body></body>"),native_forwarded:false}));
        let mut scheduler=DriverScheduler::new(host,crate::js_scheduler::JS_CAPACITY).unwrap();
        let nonce=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        scheduler.set_location_url(format!("https://storage-{nonce}.example.test/"));
        scheduler
    }

    #[test]
    fn web_storage_named_properties_and_methods_share_ordered_area() {
        let mut scheduler=scheduler();
        for script in ["localStorage.clear()","localStorage.setItem('first','one')","localStorage.second='two'"] {
            scheduler.run_script(script).unwrap_or_else(|error|panic!("{script}: {}",error.describe()));
        }
        let value=scheduler.run_script("localStorage.first==='one' && localStorage.getItem('second')==='two' && localStorage.length===2 && localStorage.key(0)==='first' && localStorage.key(1)==='second'").unwrap_or_else(|error|panic!("read: {}",error.describe()));
        assert_eq!(value,JsValue::Bool(true));
    }

    #[test]
    fn local_storage_changes_are_visible_in_another_document() {
        let _shared=SharedTestStorage::new();
        let mut first=scheduler();let mut second=scheduler();
        let origin=format!("https://shared-{}.example.test/",std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos());
        first.set_location_url(&origin);second.set_location_url(&origin);
        first.run_script("localStorage.setItem('shared','value')").unwrap();
        assert_eq!(second.run_script("localStorage.getItem('shared')").unwrap(),JsValue::str("value"));
    }

    #[test]
    fn navigator_storage_estimates_and_remembers_persistence() {
        let mut scheduler=scheduler();
        scheduler.run_script("var storageResult='pending'; navigator.storage.estimate().then(function(e){storageResult=String(e.usage)+':'+String(e.quota)});").unwrap();
        for step in 1..5 {scheduler.pump_at(Duration::from_millis(step*10),false).unwrap();}
        let estimate=scheduler.run_script("storageResult").unwrap();
        assert!(matches!(estimate,JsValue::Str(ref value) if value.to_utf8_lossy().contains(':')));
        scheduler.run_script("navigator.storage.persist().then(function(granted){storageResult=String(granted)});").unwrap();
        for step in 5..9 {scheduler.pump_at(Duration::from_millis(step*10),false).unwrap();}
        assert_eq!(scheduler.run_script("storageResult").unwrap(),JsValue::str("true"));
        scheduler.run_script("navigator.storage.persisted().then(function(value){storageResult=String(value)});").unwrap();
        for step in 9..13 {scheduler.pump_at(Duration::from_millis(step*10),false).unwrap();}
        assert_eq!(scheduler.run_script("storageResult").unwrap(),JsValue::str("true"));
    }

    #[test]
    fn web_locks_coordinate_two_documents_and_release_after_callback_promise() {
        let _shared=SharedTestStorage::new();
        let mut first=scheduler();let mut second=scheduler();
        let origin=format!("https://locks-{}.example.test/",std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos());
        first.set_location_url(&origin);second.set_location_url(&origin);
        first.run_script("var releaseLock; var firstState='pending'; navigator.locks.request('resource',function(lock){firstState=lock.name+':'+lock.mode; return new Promise(function(resolve){releaseLock=resolve;});});").unwrap();
        for step in 1..5 {first.pump_at(Duration::from_millis(step*10),false).unwrap();}
        assert_eq!(first.run_script("firstState").unwrap(),JsValue::str("resource:exclusive"));
        second.run_script("var secondState='pending'; navigator.locks.request('resource',function(lock){secondState=lock.name+':'+lock.mode;});").unwrap();
        for step in 1..5 {second.pump_at(Duration::from_millis(step*10),false).unwrap();}
        assert_eq!(second.run_script("secondState").unwrap(),JsValue::str("pending"));
        second.run_script("var lockSnapshot='pending'; var unavailable=false; navigator.locks.query().then(function(s){lockSnapshot=s.held.length+':'+s.pending.length}); navigator.locks.request('resource',{ifAvailable:true},function(lock){unavailable=lock===null;});").unwrap();
        for step in 5..9 {second.pump_at(Duration::from_millis(step*10),false).unwrap();}
        assert_eq!(second.run_script("lockSnapshot").unwrap(),JsValue::str("1:1"));
        assert_eq!(second.run_script("unavailable").unwrap(),JsValue::Bool(true));
        first.run_script("releaseLock('done')").unwrap();
        for step in 9..15 {first.pump_at(Duration::from_millis(step*10),false).unwrap();second.pump_at(Duration::from_millis(step*10),false).unwrap();}
        assert_eq!(second.run_script("secondState").unwrap(),JsValue::str("resource:exclusive"));
    }

    #[test]
    fn local_storage_events_run_on_other_same_origin_document_tasks() {
        let _shared=SharedTestStorage::new();
        let mut source=scheduler();let mut peer=scheduler();let mut other=scheduler();
        let origin=format!("https://events-{}.example.test/",std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos());
        source.set_location_url(format!("{origin}source"));
        peer.set_location_url(format!("{origin}peer"));
        other.set_location_url("https://unrelated.example.test/");
        for scheduler in [&mut source,&mut peer,&mut other] {
            scheduler.run_script("var events=[]; var propertyEvents=[]; var listener=function(e){events.push([e.key,e.oldValue,e.newValue,e.url,e.storageArea===localStorage,e.target===window])}; window.addEventListener('storage',listener); window.onstorage=function(e){propertyEvents.push(e.key)};").unwrap();
        }
        source.run_script("localStorage.setItem('name','first')").unwrap();
        assert_eq!(source.run_script("events.length").unwrap(),JsValue::num(0.0));
        assert_eq!(peer.run_script("events.length").unwrap(),JsValue::num(0.0));
        peer.pump_at(Duration::from_millis(1),false).unwrap();
        other.pump_at(Duration::from_millis(1),false).unwrap();
        assert_eq!(peer.run_script("JSON.stringify(events)").unwrap(),
            JsValue::str(format!("[[\"name\",null,\"first\",\"{origin}source\",true,true]]")));
        assert_eq!(other.run_script("events.length").unwrap(),JsValue::num(0.0));
        assert_eq!(peer.run_script("propertyEvents[0]").unwrap(),JsValue::str("name"));
        source.run_script("localStorage.setItem('name','first')").unwrap();
        peer.pump_at(Duration::from_millis(2),false).unwrap();
        assert_eq!(peer.run_script("events.length").unwrap(),JsValue::num(1.0));
        source.run_script("localStorage.removeItem('name')").unwrap();
        peer.pump_at(Duration::from_millis(3),false).unwrap();
        assert_eq!(peer.run_script("JSON.stringify(events[1].slice(0,3))").unwrap(),
            JsValue::str("[\"name\",\"first\",null]"));
        source.run_script("localStorage.setItem('again','value'); localStorage.clear()").unwrap();
        peer.pump_at(Duration::from_millis(4),false).unwrap();
        assert_eq!(peer.run_script("JSON.stringify(events[3].slice(0,3))").unwrap(),
            JsValue::str("[null,null,null]"));
        peer.run_script("window.removeEventListener('storage',listener); window.onstorage=null").unwrap();
        source.run_script("localStorage.setItem('last','value')").unwrap();
        peer.pump_at(Duration::from_millis(5),false).unwrap();
        assert_eq!(peer.run_script("events.length").unwrap(),JsValue::num(4.0));
    }

    #[test]
    fn page_load_and_first_storage_use_survive_a_corrupt_on_disk_store() {
        // IA-REAL-2: install_storage_interfaces() used to eagerly open every
        // on-disk store (web/idb/cache/manager/buckets) during realm init, so
        // a corrupt or unrecognized-format store on disk made every page fail
        // before any script ran ("TypeError: IndexedDB open: Corrupt"), even
        // for a page that never touches storage. Storage now opens lazily on
        // first indexedDB/caches/navigator.storage use, and a corrupt or
        // unknown-version store is quarantined instead of failing.
        let shared=SharedTestStorage::new();
        let idb_dir=shared._root.path.join("idb");
        std::fs::create_dir_all(&idb_dir).unwrap();
        // A checkpoint file that exists but fails to parse: `saw_checkpoint`
        // true and `generation` stuck at 0, the exact condition that used to
        // make `DiskStore::open` return `Error::Corrupt`.
        std::fs::write(idb_dir.join("checkpoint.0"),b"not a real checkpoint").unwrap();
        // Page (realm) init must succeed even though the on-disk IndexedDB
        // store next to it is corrupt: init no longer touches storage at all.
        let mut scheduler=scheduler();
        // First actual use of indexedDB must still work, against a fresh store.
        scheduler.run_script("var result='pending'; var request=indexedDB.open('after-quarantine',1); request.onupgradeneeded=function(){request.result.createObjectStore('items')}; request.onsuccess=function(){result='ok'}; request.onerror=function(){result='error: '+request.error.name};").unwrap();
        for step in 1..15 {
            scheduler.pump_at(Duration::from_millis(step*10),false).unwrap_or_else(|error|panic!("pump: {}",error.describe()));
            if scheduler.run_script("result").unwrap()!=JsValue::str("pending") {break}
        }
        assert_eq!(scheduler.run_script("result").unwrap(),JsValue::str("ok"));
        let quarantined=std::fs::read_dir(&shared._root.path).unwrap()
            .filter_map(|entry|entry.ok())
            .any(|entry|entry.file_name().to_string_lossy().starts_with("idb.corrupt-"));
        assert!(quarantined,"expected the corrupt idb store to be quarantined aside, not left in place or deleted");
    }

    #[test]
    fn indexeddb_upgrade_request_commits_then_reads_record() {
        let mut scheduler=scheduler();
        scheduler.run_script("var result='pending'; var phase='start'; var request=indexedDB.open('integration',1); request.onupgradeneeded=function(){ phase='upgrade'; var db=request.result; db.createObjectStore('items'); }; request.onsuccess=function(){ phase='success'; try { var tx=request.result.transaction('items','readwrite'); phase='transaction'; var store=tx.objectStore('items'); phase='store'; var put=store.put({label:'stored'},7); phase='put-request'; put.onsuccess=function(){ result='put'; }; tx.oncomplete=function(){ phase='complete'; var read=request.result.transaction('items'); var get=read.objectStore('items').get(7); get.onsuccess=function(){ result=get.result.label; }; }; } catch(e) {result=e.name+':'+e.message;} }; request.onerror=function(){result='error';};").unwrap();
        for step in 1..20 {
            scheduler.pump_at(Duration::from_millis(step*10),false).unwrap_or_else(|error|{
                let phase=scheduler.run_script("phase+':'+result").unwrap();
                let details=match error.0 {JsValue::Object(object)=>{let object=object.borrow();format!("{:?} {:?}",object.get_own("name"),object.get_own("message"))},other=>format!("{other:?}")};
                panic!("pump at {phase:?}: {details}")
            });
            if scheduler.run_script("result").unwrap()==JsValue::str("stored") {break}
        }
        assert_eq!(scheduler.run_script("result+':'+phase+':'+request.readyState").unwrap(),JsValue::str("stored:complete:done"));
    }

    #[test]
    fn indexeddb_structured_clone_preserves_cycles_and_shared_identity() {
        let mut scheduler=scheduler();
        scheduler.run_script("var cloneResult='pending'; var open=indexedDB.open('graph',1); open.onupgradeneeded=function(){open.result.createObjectStore('items')}; open.onsuccess=function(){var shared={value:7}; var original={left:shared,right:shared}; original.self=original; var tx=open.result.transaction('items','readwrite'); tx.objectStore('items').put(original,1); tx.oncomplete=function(){var read=open.result.transaction('items').objectStore('items').get(1); read.onsuccess=function(){var value=read.result; cloneResult=String(value!==original && value.self===value && value.left===value.right && value.left.value===7)};};};").unwrap();
        for step in 1..30 { scheduler.pump_at(Duration::from_millis(step*10),false).unwrap(); if scheduler.run_script("cloneResult").unwrap()!=JsValue::str("pending") {break} }
        assert_eq!(scheduler.run_script("cloneResult").unwrap(),JsValue::str("true"));
    }

    #[test]
    fn indexeddb_compares_binary_date_and_array_keys() {
        let mut scheduler=scheduler();
        for (script, expected) in [
            ("indexedDB.cmp([0], new Uint8Array([0]))", 1.0),
            ("indexedDB.cmp(new Uint8Array([0]), '0')", 1.0),
            ("indexedDB.cmp('', new Date(0))", 1.0),
            ("indexedDB.cmp(new Date(0), 0)", 1.0),
            ("indexedDB.cmp(new Int8Array([-1]), new Uint8Array([0]))", 1.0),
        ] {
            assert_eq!(scheduler.run_script(script).unwrap_or_else(|e| panic!("{script}: {}",e.describe())),JsValue::num(expected));
        }
    }

    #[test]
    fn indexeddb_array_key_conversion_reads_own_elements_only() {
        let mut scheduler=scheduler();
        // A hole at index 50 must not run an inherited accessor, and the lone
        // own element at 99 must still convert; the array key then compares equal.
        let value=scheduler.run_script("var calls=0; Object.defineProperty(Object.prototype,'50',{enumerable:true,configurable:true,get:function(){calls++;return 'foo';}}); var array=[]; array[99]=1; var cmp=indexedDB.cmp(array,array); delete Object.prototype['50']; calls+':'+cmp").unwrap_or_else(|error|panic!("{}",error.describe()));
        assert_eq!(value,JsValue::str("0:0"));
    }

    #[test]
    fn indexeddb_clone_invokes_key_path_getters_once() {
        let mut scheduler=scheduler();
        // WPT clone-before-keypath-eval shape: an accessor-backed key path must
        // be evaluated against the structured clone, so the source getter runs
        // exactly once and put() does not throw.
        scheduler.run_script("function Probe(){this.id_count=0;this.prop_count=0;Object.defineProperties(this,{id:{enumerable:true,get:function(){++this.id_count;return 1000+this.id_count;}},prop:{enumerable:true,get:function(){++this.prop_count;return 2000+this.prop_count;}}});} \
            var result='pending'; var rq=indexedDB.open('clone-kp',1); \
            rq.onupgradeneeded=function(){rq.result.createObjectStore('store',{keyPath:'id'});}; \
            rq.onsuccess=function(){try{var tx=rq.result.transaction('store','readwrite');var o=new Probe();tx.objectStore('store').put(o);result=o.id_count+','+o.prop_count;}catch(e){result='throw:'+e.name+':'+e.message;}};").unwrap();
        for step in 1..20 {
            scheduler.pump_at(Duration::from_millis(step*10),false).unwrap();
            if scheduler.run_script("result").unwrap()!=JsValue::str("pending") {break}
        }
        assert_eq!(scheduler.run_script("result").unwrap(),JsValue::str("1,1"));
    }

    #[test]
    fn indexeddb_cursor_advance_under_realtime_pump() {
        let mut scheduler=scheduler();
        // Mirrors the WPT harness loop: pump_now with a real clock and a 1 ms
        // sleep, with no explicit pump_at deadline. index cursor over five
        // records, advance(2) per success.
        scheduler.run_script("var out=[]; var rq=indexedDB.open('advance-realtime',1); \
            rq.onupgradeneeded=function(){var s=rq.result.createObjectStore('t');s.createIndex('i','');s.add('cupcake',5);s.add('pancake',3);s.add('pie',1);s.add('pie',4);s.add('taco',2);}; \
            rq.onsuccess=function(){var c=rq.result.transaction('t','readonly').objectStore('t').index('i').openCursor(); \
            c.onsuccess=function(e){var cur=e.target.result;if(!cur){out.push('end');return;}out.push(String(cur.primaryKey));cur.advance(2);};};").unwrap();
        for _ in 0..300 {
            scheduler.pump_now(false).unwrap();
            std::thread::sleep(Duration::from_millis(1));
            if scheduler.run_script("out.length").unwrap()==JsValue::num(4.0) {break}
        }
        assert_eq!(scheduler.run_script("out.join(',')").unwrap(),JsValue::str("5,1,2,end"));
    }

    #[test]
    fn indexeddb_preserves_undefined_through_commit() {
        let mut scheduler=scheduler();
        scheduler.run_script("var result='pending'; var rq=indexedDB.open('undefined-value',1); rq.onupgradeneeded=function(){rq.result.createObjectStore('items')}; rq.onsuccess=function(){var tx=rq.result.transaction('items','readwrite'); tx.objectStore('items').put(undefined,1); tx.oncomplete=function(){var read=rq.result.transaction('items'); var get=read.objectStore('items').get(1); get.onsuccess=function(){result=get.result===undefined?'undefined':'wrong'};};};").unwrap();
        for step in 1..20 {
            scheduler.pump_at(Duration::from_millis(step*10),false).unwrap();
            if scheduler.run_script("result").unwrap()==JsValue::str("undefined") {break}
        }
        assert_eq!(scheduler.run_script("result").unwrap(),JsValue::str("undefined"));
    }

    #[test]
    fn indexeddb_compound_key_path_round_trips() {
        let mut scheduler=scheduler();
        scheduler.run_script("var result='pending'; var rq=indexedDB.open('compound',1); rq.onupgradeneeded=function(){var store=rq.result.createObjectStore('items',{keyPath:['group','id']}); result=store.keyPath.join(','); store.put({group:'a',id:2,value:'found'});}; rq.onsuccess=function(){var get=rq.result.transaction('items').objectStore('items').get(['a',2]); get.onsuccess=function(){result+=':'+get.result.value};};").unwrap();
        for step in 1..20 {
            scheduler.pump_at(Duration::from_millis(step*10),false).unwrap();
            if scheduler.run_script("result").unwrap()==JsValue::str("group,id:found") {break}
        }
        assert_eq!(scheduler.run_script("result").unwrap(),JsValue::str("group,id:found"));
    }

    #[test]
    fn indexeddb_blocked_upgrade_resumes_after_close() {
        let mut scheduler=scheduler();
        scheduler.run_script("var result='pending'; var first=indexedDB.open('upgrade',1); first.onupgradeneeded=function(){first.result.createObjectStore('old')}; first.onsuccess=function(){var db=first.result; db.onversionchange=function(e){result='version:'+e.oldVersion+':'+e.newVersion;db.close()}; var second=indexedDB.open('upgrade',2); second.onupgradeneeded=function(){second.result.createObjectStore('new')}; second.onsuccess=function(){result+=':ready:'+second.result.version};};").unwrap();
        for step in 1..30 {
            scheduler.pump_at(Duration::from_millis(step*10),false).unwrap();
            if scheduler.run_script("result").unwrap()==JsValue::str("version:1:2:ready:2") {break}
        }
        assert_eq!(scheduler.run_script("result").unwrap(),JsValue::str("version:1:2:ready:2"));
    }

    #[test]
    fn storage_buckets_open_keys_describe_and_invalidate() {
        let mut scheduler=scheduler();
        scheduler.run_script("var log=[]; var bucket=null; \
            navigator.storageBuckets.open('alpha',{durability:'strict',quota:4096,expires:777}) \
            .then(function(b){bucket=b;log.push(b.name);return navigator.storageBuckets.open('beta')}) \
            .then(function(){return navigator.storageBuckets.keys()}) \
            .then(function(names){log.push(names.join(','));return bucket.durability()}) \
            .then(function(d){log.push(d);return bucket.expires()}) \
            .then(function(e){log.push(String(e));return bucket.estimate()}) \
            .then(function(est){log.push(est.quota+':'+est.usage);log.push(typeof bucket.indexedDB.open);return navigator.storageBuckets.delete('alpha')}) \
            .then(function(){return bucket.persisted()}) \
            .catch(function(error){log.push('error:'+error.name)});").unwrap();
        for step in 1..25 { scheduler.pump_at(Duration::from_millis(step*10),false).unwrap(); }
        assert_eq!(scheduler.run_script("log.join('|')").unwrap(),
            JsValue::str("alpha|alpha,beta|strict|777|4096:0|function|error:UnknownError"));
    }

    #[test]
    fn storage_buckets_reject_invalid_names_and_quotas() {
        let mut scheduler=scheduler();
        scheduler.run_script("var rejected=[]; function capture(p){return p.then(function(){rejected.push('resolved')},function(e){rejected.push(e.name)})} \
            capture(navigator.storageBuckets.open('Bad Name')); \
            capture(navigator.storageBuckets.open('_bad')); \
            capture(navigator.storageBuckets.open('ok',{quota:0})); \
            capture(navigator.storageBuckets.open('ok',{quota:9007199254740992})); \
            capture(navigator.storageBuckets.delete('bad name'));").unwrap();
        for step in 1..15 { scheduler.pump_at(Duration::from_millis(step*10),false).unwrap(); }
        assert_eq!(scheduler.run_script("rejected.join(',')").unwrap(),
            JsValue::str("TypeError,TypeError,TypeError,TypeError,TypeError"));
    }

    // Regression for the STORE-6 bucket-scoping bisection: c1e29741 replaced the cached
    // BucketKey on StorageObject::Database with a lookup through the live idb::Connection,
    // so *every* IDBDatabase member access (including the spec-idempotent second
    // `close()` call, and read-only getters like `name`/`objectStoreNames` that must keep
    // working after close) threw an uncaught "IDBDatabase is closed" TypeError once the
    // connection was closed. WPT's IndexedDB support helpers routinely call `db.close()`
    // defensively during per-subtest cleanup and read `db.name` for diagnostics; the
    // uncaught throw from that cleanup step left the harness with no completion signal, so
    // the first subtest in a file passed and every later subtest in that file timed out.
    // This exercises the same sequence directly against the driver, with no harness
    // involved, so it fails fast (as an Err/panic) instead of hanging.
    #[test]
    fn indexeddb_database_members_survive_close() {
        let mut scheduler=scheduler();
        scheduler.run_script("var log=[]; \
            var open=indexedDB.open('members-survive-close',1); \
            open.onupgradeneeded=function(){open.result.createObjectStore('items')}; \
            open.onsuccess=function(){ \
                var db=open.result; \
                db.close(); \
                log.push(db.name); \
                log.push(db.objectStoreNames.length+':'+db.objectStoreNames[0]); \
                db.close(); \
                log.push('ok'); \
            };").unwrap();
        for step in 1..20 { scheduler.pump_at(Duration::from_millis(step*10),false).unwrap(); }
        assert_eq!(scheduler.run_script("log.join('|')").unwrap(),
            JsValue::str("members-survive-close|1:items|ok"));
    }

    // STORE-7 triage: IndexedDB/idbcursor-direction.any.js opens five independent,
    // uniquely-named databases synchronously (one per `cursor_direction(...)` call) before
    // any task runs, each with its own upgradeneeded/success/cursor flow. All five report
    // "Object store not found" against the current dev tree. Reproduces the minimal shape
    // (two concurrent independent opens) directly against the driver to see whether the
    // failure is a real concurrent-open bug or specific to the harness/support.js plumbing.
    #[test]
    fn indexeddb_concurrent_independent_opens_each_see_their_own_store() {
        let mut scheduler=scheduler();
        scheduler.run_script("var log=[]; \
            function openone(name,tag){ \
                var open=indexedDB.open(name,1); \
                open.onupgradeneeded=function(){ \
                    var os=open.result.createObjectStore('test'); \
                    os.add('data','key'); \
                }; \
                open.onerror=function(){log.push(tag+':open-error:'+open.error.name)}; \
                open.onsuccess=function(){ \
                    var db=open.result; \
                    try { \
                        var rq=db.transaction('test','readonly').objectStore('test').openCursor(); \
                        rq.onsuccess=function(e){ \
                            var cursor=e.target.result; \
                            log.push(tag+':'+(cursor?cursor.value:'no-cursor')); \
                        }; \
                        rq.onerror=function(){log.push(tag+':cursor-error:'+rq.error.name)}; \
                    } catch (e) { log.push(tag+':throw:'+e.name); } \
                }; \
            } \
            openone('concurrent-a','a'); \
            openone('concurrent-b','b');").unwrap();
        for step in 1..30 { scheduler.pump_at(Duration::from_millis(step*10),false).unwrap(); }
        let mut log=scheduler.run_script("log.slice().sort().join('|')").unwrap_or_else(|error|panic!("read log: {}",error.describe()));
        if log==JsValue::Undefined { log=JsValue::str(""); }
        assert_eq!(log,JsValue::str("a:data|b:data"));
    }

    // STORE-7 triage: WPT's IndexedDB/resources/support.js `createdb_for_multiple_tests`
    // helper installs a *custom* accessor for "onupgradeneeded"/"onsuccess"/etc. on each
    // IDBOpenDBRequest via `__defineSetter__`, wrapping every assignment in an
    // addEventListener call and flipping a `manually_handled` flag that a separately
    // pre-registered "fail if nothing handled this event" listener checks. This only works
    // if our engine's property SET for "on"+event on a native IDBOpenDBRequest first
    // consults the *object's own* JS-defined accessor (installed via defineProperty/
    // __defineSetter__) instead of going straight to storage_idb_invoke's native
    // `handlers` map. Several pinned WPT files fail with "assert_unreached: unexpected
    // open.<event> event", which is exactly the symptom of the manually_handled flag never
    // getting set -- i.e. the custom setter never running.
    #[test]
    fn indexeddb_open_request_on_event_setter_honors_own_defined_accessor() {
        let mut scheduler=scheduler();
        scheduler.run_script("var log=[]; \
            var open=indexedDB.open('on-event-setter-override',1); \
            var seen=false; \
            Object.defineProperty(open,'onupgradeneeded',{set:function(h){ \
                seen=true; \
                open.addEventListener('upgradeneeded',h); \
            }}); \
            open.onupgradeneeded=function(){log.push('own-accessor-ran:'+seen);open.result.createObjectStore('s');};").unwrap();
        for step in 1..20 { scheduler.pump_at(Duration::from_millis(step*10),false).unwrap(); }
        assert_eq!(scheduler.run_script("log.join('|')").unwrap_or_else(|error|panic!("read log: {}",error.describe())),
            JsValue::str("own-accessor-ran:true"));
    }

    // STORE-7 regression: root cause of the "Object store not found" cluster
    // (idbcursor-direction.any.js, idbcursor_{update,delete}_objectstore.any.js,
    // idbindex_get(Key).any.js). `OpenUpgrade` used to schedule the open
    // request's "success" job unconditionally, right after scheduling its
    // versionchange transaction's `TransactionComplete`. `TransactionComplete`
    // defers itself (reschedules to the *back* of the queue) whenever a request
    // placed against the transaction -- like the `add()` a real upgradeneeded
    // handler makes -- hasn't been delivered yet. That reschedule landed behind
    // the open request's already-queued RequestSuccess job, so "success" fired
    // -- and ran the test's `db.transaction(...)` -- before the transaction that
    // created the store had actually committed. Fixed by having
    // `TransactionComplete`'s commit branch schedule RequestSuccess itself, the
    // same way its abort branch already schedules RequestError. Reproduces two
    // independent `createdb`-shaped opens wired the way WPT's
    // resources/support.js wires them (`auto_fail` + `addEventListener` +
    // Object.defineProperty on "on<event>"), each running the same
    // two-openCursor-then-done shape as `cursor_direction()`.
    #[test]
    fn indexeddb_two_independent_auto_fail_wired_cursor_tests_both_complete() {
        let mut scheduler=scheduler();
        scheduler.run_script("var log=[]; \
            function auto_fail(rq,evt){ \
                rq.__handled=rq.__handled||{}; \
                rq.addEventListener(evt,function(e){ \
                    if(!rq.__handled[evt]) log.push('UNREACHED:'+evt); \
                }); \
                Object.defineProperty(rq,'on'+evt,{configurable:true,set:function(h){ \
                    rq.__handled[evt]=true; \
                    if(h) rq.addEventListener(evt,h); \
                }}); \
            } \
            function createdb(name){ \
                var rq=indexedDB.open(name,1); \
                ['upgradeneeded','success','blocked','error'].forEach(function(evt){auto_fail(rq,evt)}); \
                return rq; \
            } \
            function cursor_direction(name,tag){ \
                var db; \
                var rq=createdb(name); \
                rq.onupgradeneeded=function(e){ \
                    db=e.target.result; \
                    var os=db.createObjectStore('test'); \
                    os.add('data','key'); \
                }; \
                rq.onsuccess=function(e){ \
                    var count=0; \
                    var os=db.transaction('test','readonly').objectStore('test'); \
                    var rq1=os.openCursor(); \
                    rq1.onsuccess=function(e){ \
                        var cursor=e.target.result; \
                        log.push(tag+':1:'+(cursor?cursor.value:'null')); \
                        count++; if(count>=2) log.push(tag+':done'); \
                    }; \
                    var rq2=db.transaction('test','readonly').objectStore('test').openCursor(); \
                    rq2.onsuccess=function(e){ \
                        var cursor=e.target.result; \
                        log.push(tag+':2:'+(cursor?cursor.value:'null')); \
                        count++; if(count>=2) log.push(tag+':done'); \
                    }; \
                }; \
            } \
            cursor_direction('cd-a','a'); \
            cursor_direction('cd-b','b');").unwrap();
        for step in 1..40 {
            scheduler.pump_at(Duration::from_millis(step*10),false).unwrap_or_else(|error|{
                let log=scheduler.run_script("log.join('|')").unwrap_or_else(|_|JsValue::str("<unreadable>"));
                panic!("pump step {step} threw {}; log so far: {log:?}",error.describe());
            });
        }
        let log=scheduler.run_script("log.slice().sort().join('|')").unwrap_or_else(|error|panic!("read log: {}",error.describe()));
        assert_eq!(log,JsValue::str("a:1:data|a:2:data|a:done|b:1:data|b:2:data|b:done"));
    }
}
