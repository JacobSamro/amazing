//! Web Storage, IndexedDB, and Cache API driver bindings.
use super::*;
use std::{collections::{BTreeMap,VecDeque}, path::PathBuf, rc::Weak};
use amazing_bindings::storage_interfaces;
use amazing_storage::{core::{BucketKey, DiskStore, StorageKey}, web::{PersistentWebStorage, StorageChange}, idb::{self, IndexedDb, StructuredValue, Key, KeyRange, Direction, Mode, Durability, Transaction as IdbTransaction, Cursor as IdbCursor}, cache::{self, PersistentCacheStorage}};
use std::sync::atomic::{AtomicU64, Ordering};

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
    cache: PersistentCacheStorage,
    objects: Vec<StorageObject>,
    wrappers: HashMap<usize, JsValue>,
    pending: VecDeque<StorageJob>,
    blocked_opens: VecDeque<(StorageKey,String,usize)>,
    blocked_deletes: VecDeque<(StorageKey,String,usize)>,
    tick_scheduled: bool,
    _test_root: Option<Rc<TestStorageRoot>>,
}

thread_local! {
    static SHARED_WEB_STORAGE: RefCell<HashMap<PathBuf,Weak<RefCell<PersistentWebStorage>>>>=RefCell::new(HashMap::new());
    static SHARED_INDEXED_DB: RefCell<HashMap<PathBuf,Weak<RefCell<IndexedDb>>>>=RefCell::new(HashMap::new());
}

enum StorageObject {
    Area { session: bool },
    Factory,
    Request { result: JsValue, error: JsValue, ready: bool, source: JsValue, transaction: JsValue, handlers: HashMap<String, JsValue>, listeners: Vec<(String, JsValue)> },
    Database { connection: u64, name: String, handlers: HashMap<String, JsValue>, listeners: Vec<(String, JsValue)> },
    Transaction { connection: u64, id: u64, value: Option<IdbTransaction>, mode: Mode, scope: Vec<String>, queued: bool, active_for_requests: bool, handlers: HashMap<String, JsValue>, listeners: Vec<(String, JsValue)> },
    Store { transaction: usize, name: String },
    Index { transaction: usize, store: String, name: String },
    Cursor { cursor: IdbCursor, transaction: usize, source: usize, request: usize, with_value: bool },
    Range(KeyRange),
    CacheStorage,
    Cache { name: String },
}

enum StorageJob {
    RequestSuccess { request: usize, result: JsValue },
    RequestError { request: usize, error: JsValue },
    DeleteSuccess { request: usize, old: u64 },
    OpenUpgrade { request: usize, connection: u64, name: String, old: u64, new: u64 },
    OpenBlocked { request: usize, old: u64, new: Option<u64> },
    DatabaseVersionChange { connection: u64, old: u64, new: Option<u64> },
    DispatchListener { slot: usize, event: JsValue, callbacks: VecDeque<JsValue> },
    TransactionComplete { transaction: usize },
    StorageEvent(StorageChange),
}

enum StoreResult {
    None,
    Count(usize),
    Key(Key),
    OptionalKey(Option<Key>),
    Keys(Vec<Key>),
    Value(Option<StructuredValue>),
    Values(Vec<StructuredValue>),
}

impl RuntimeBindings {
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
        for (name,interface,object) in [
            ("indexedDB",SI::IDBFactory,StorageObject::Factory),
            ("caches",SI::CacheStorage,StorageObject::CacheStorage),
        ] {
            let wrapper=self.storage_allocate(interface,object)?;
            self.define_data_property(&global,name,wrapper,true,true,true)?;
        }
        self.install_storage_legacy();
        Ok(())
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
        let web=SHARED_WEB_STORAGE.with(|shared|->Result<_,Throw>{
            if let Some(value)=shared.borrow().get(&path).and_then(Weak::upgrade){return Ok(value)}
            let value=Rc::new(RefCell::new(PersistentWebStorage::open(DiskStore::open(path.join("web"),STORAGE_QUOTA)
                .map_err(|e|Throw::type_error(format!("storage open: {e:?}")))?,10*1024*1024)
                .map_err(|e|Throw::type_error(format!("Web Storage open: {e:?}")))?));
            shared.borrow_mut().insert(path.clone(),Rc::downgrade(&value));Ok(value)
        })?;
        let idb=SHARED_INDEXED_DB.with(|shared|->Result<_,Throw>{
            if let Some(value)=shared.borrow().get(&path).and_then(Weak::upgrade){return Ok(value)}
            let value=Rc::new(RefCell::new(IndexedDb::with_disk(DiskStore::open(path.join("idb"),STORAGE_QUOTA)
                .map_err(|e|Throw::type_error(format!("IndexedDB open: {e:?}")))?)
                .map_err(|e|Throw::type_error(format!("IndexedDB decode: {e:?}")))?));
            shared.borrow_mut().insert(path.clone(),Rc::downgrade(&value));Ok(value)
        })?;
        let cache=PersistentCacheStorage::open(DiskStore::open(path.join("cache"),STORAGE_QUOTA)
            .map_err(|e|Throw::type_error(format!("Cache open: {e:?}")))?)
            .map_err(|e|Throw::type_error(format!("Cache decode: {e:?}")))?;
        self.state.borrow_mut().storage=Some(StorageRuntime {web,idb,cache,objects:Vec::new(),wrappers:HashMap::new(),pending:VecDeque::new(),blocked_opens:VecDeque::new(),blocked_deletes:VecDeque::new(),tick_scheduled:false,_test_root:test_root});
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
        self.materialize_accessors(&value);
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
                let area=web.session(id,key);
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
            Ok(Some(_change))=>{},
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
    fn storage_request(&mut self, open: bool, source: JsValue, transaction: JsValue) -> Result<(usize, JsValue), Throw> {
        let kind=if open {SI::IDBOpenDBRequest}else{SI::IDBRequest};
        let value=self.storage_allocate(kind,StorageObject::Request {result:JsValue::Undefined,error:JsValue::Null,
            ready:false,source,transaction,handlers:HashMap::new(),listeners:Vec::new()})?;
        let slot=self.storage_slot(&value,kind)?;
        Ok((slot,value))
    }
    fn storage_factory_invoke(&mut self, member: &str, this: JsValue, args: &[JsValue]) -> Result<JsValue, Throw> {
        let _=self.storage_slot(&this,SI::IDBFactory)?;
        let key=self.storage_key()?;
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
                        if open.blocked {self.state.borrow_mut().storage.as_mut().unwrap().blocked_opens.push_back((key,name.clone(),request));}
                        for event in open.events {
                            match event {
                                idb::OpenEvent::UpgradeNeeded{old,new}=>{
                                    self.storage_schedule(StorageJob::OpenUpgrade{request,connection:open.connection.unwrap(),name:name.clone(),old,new})?;
                                },
                                idb::OpenEvent::Blocked{old,new}=>{
                                    self.storage_schedule(StorageJob::OpenBlocked{request,old,new})?;
                                },
                                idb::OpenEvent::Success{connection,..}=>{
                                    let database=self.storage_allocate(SI::IDBDatabase,StorageObject::Database{connection,name:name.clone(),handlers:HashMap::new(),listeners:Vec::new()})?;
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
                        self.storage_schedule(StorageJob::OpenUpgrade{request,connection,name:name.clone(),old,new})?;
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
                "lowerBound"=>KeyRange{lower:Some(a),upper:None,lower_open:args.get(1).is_some_and(|v|self.js.binding_to_boolean(v)),upper_open:false},
                "upperBound"=>KeyRange{lower:None,upper:Some(a),lower_open:false,upper_open:args.get(1).is_some_and(|v|self.js.binding_to_boolean(v))},
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
                let (connection,name)={let state=self.state.borrow();match &state.storage.as_ref().unwrap().objects[slot]{StorageObject::Database{connection,name,..}=>(*connection,name.clone()),_=>unreachable!()}};
                let key=self.storage_key()?;
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
                        {let mut state=self.state.borrow_mut();let StorageObject::Transaction{value:Some(tx),..}=&mut state.storage.as_mut().unwrap().objects[transaction] else{unreachable!()};
                            tx.database_mut().and_then(|db|db.create_store(name.clone(),key_path,auto)).map_err(|error|Throw::type_error(format!("{error:?}")))?;}
                        self.storage_allocate(SI::IDBObjectStore,StorageObject::Store{transaction,name})
                    },
                    "deleteObjectStore"=>{let name=self.storage_string(self.storage_arg(args,0)?)?;let transaction=self.storage_upgrade_transaction(connection)?;
                        {let mut state=self.state.borrow_mut();let StorageObject::Transaction{value:Some(tx),..}=&mut state.storage.as_mut().unwrap().objects[transaction] else{unreachable!()};
                            tx.database_mut().and_then(|db|db.delete_store(&name)).map_err(|error|Throw::type_error(format!("{error:?}")))?;}Ok(JsValue::Undefined)},
                    _=>Ok(JsValue::Null),
                }
            },
            SI::IDBTransaction=>self.storage_transaction_invoke(member,kind,this,args),
            SI::IDBObjectStore=>self.storage_store_invoke(member,this,args),
            SI::IDBIndex=>self.storage_index_invoke(member,this,args),
            SI::IDBCursor|SI::IDBCursorWithValue=>self.storage_cursor_invoke(interface,member,this,args),
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
        let (connection,mode,names,active)={let state=self.state.borrow();let StorageObject::Transaction{connection,mode,scope,value,queued,..}=&state.storage.as_ref().unwrap().objects[slot] else{unreachable!()};
            (*connection,*mode,scope.clone(),value.is_some()||*queued)};
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
                self.storage_allocate(SI::IDBObjectStore,StorageObject::Store{transaction:slot,name})},
            "abort"=>{let mut state=self.state.borrow_mut();if let StorageObject::Transaction{value:Some(tx),..}=&mut state.storage.as_mut().unwrap().objects[slot]{tx.abort();}Ok(JsValue::Undefined)},
            "commit"=>Ok(JsValue::Undefined),
            _=>Err(Throw::type_error("Unknown IDBTransaction member")),
        }
    }
    fn storage_store_invoke(&mut self,member:&str,this:JsValue,args:&[JsValue])->Result<JsValue,Throw>{
        let slot=self.storage_slot(&this,SI::IDBObjectStore)?;
        let (transaction,name)={let state=self.state.borrow();match &state.storage.as_ref().unwrap().objects[slot]{StorageObject::Store{transaction,name}=>(*transaction,name.clone()),_=>unreachable!()}};
        match member {
            "name"=>return Ok(JsValue::str(name)),"transaction"=>return self.storage_wrapper(transaction),
            "keyPath"|"autoIncrement"|"indexNames"=>{
                let (key_path,auto,names)={let state=self.state.borrow();let StorageObject::Transaction{value:Some(tx),..}=&state.storage.as_ref().unwrap().objects[transaction]else{return Err(Throw::type_error("Inactive transaction"))};
                    let store=tx.store(&name).map_err(|error|Throw::type_error(format!("{error:?}")))?;
                    (store.key_path.clone(),store.auto_increment,store.index_names().into_iter().map(str::to_owned).collect::<Vec<_>>())};
                return match member {"keyPath"=>key_path.as_ref().map_or(Ok(JsValue::Null),|path|self.storage_key_path_value(path)),
                    "autoIncrement"=>Ok(JsValue::Bool(auto)),_=>self.js.binding_create_array(names.into_iter().map(JsValue::str).collect())};
            },
            _=>{},
        }
        if matches!(member,"createIndex"|"index"|"deleteIndex") {
            if member!="index" {self.storage_transaction_accepts_request(transaction)?;}
            let index_name=self.storage_string(self.storage_arg(args,0)?)?;
            if member=="index" {
                {let state=self.state.borrow();let StorageObject::Transaction{value:Some(tx),..}=&state.storage.as_ref().unwrap().objects[transaction] else{return Err(Throw::type_error("Inactive transaction"))};
                    tx.store(&name).and_then(|store|store.index(&index_name)).map_err(|error|Throw::type_error(format!("{error:?}")))?;}
                return self.storage_allocate(SI::IDBIndex,StorageObject::Index{transaction,store:name,name:index_name});
            }
            if member=="createIndex" {
                let path=self.storage_parse_key_path(self.storage_arg(args,1)?)?
                    .ok_or_else(||self.storage_dom_error("SyntaxError","Index key path is required"))?;
                let options=args.get(2).cloned().unwrap_or(JsValue::Undefined);
                let unique=if matches!(options,JsValue::Object(_)){let value=self.js.binding_get_property(&options,"unique")?;self.js.binding_to_boolean(&value)}else{false};
                let multi=if matches!(options,JsValue::Object(_)){let value=self.js.binding_get_property(&options,"multiEntry")?;self.js.binding_to_boolean(&value)}else{false};
                {let mut state=self.state.borrow_mut();let StorageObject::Transaction{value:Some(tx),mode:Mode::VersionChange,..}=&mut state.storage.as_mut().unwrap().objects[transaction] else{return Err(Throw::type_error("No active upgrade transaction"))};
                    tx.store_mut(&name).and_then(|store|store.create_index(index_name.clone(),path,unique,multi)).map_err(|error|Throw::type_error(format!("{error:?}")))?;}
                return self.storage_allocate(SI::IDBIndex,StorageObject::Index{transaction,store:name,name:index_name});
            }
            {let mut state=self.state.borrow_mut();let StorageObject::Transaction{value:Some(tx),mode:Mode::VersionChange,..}=&mut state.storage.as_mut().unwrap().objects[transaction] else{return Err(Throw::type_error("No active upgrade transaction"))};
                tx.store_mut(&name).and_then(|store|store.delete_index(&index_name)).map_err(|error|Throw::type_error(format!("{error:?}")))?;}
            return Ok(JsValue::Undefined);
        }
        if member=="openCursor"||member=="openKeyCursor" {
            self.storage_transaction_accepts_request(transaction)?;
            let range=self.storage_query_range(args.first())?;
            let direction=self.storage_direction(args.get(1))?;
            let cursor={let state=self.state.borrow();let StorageObject::Transaction{value:Some(tx),..}=&state.storage.as_ref().unwrap().objects[transaction] else{return Err(Throw::type_error("Inactive transaction"))};
                tx.store(&name).map_err(|error|Throw::type_error(format!("{error:?}")))?.cursor(range.as_ref(),direction)};
            return self.storage_open_cursor(cursor,transaction,slot,this,member=="openCursor");
        }
        self.storage_transaction_accepts_request(transaction)?;
        let (key,range)=if matches!(member,"add"|"put") {
            let key=args.get(1).map(|value|self.storage_to_key(value)).transpose()?;(key,None)
        }else if let Some(value)=args.first(){
            if let Some(handle)=self.js.binding_host_handle(value,STORAGE_BRAND_BASE+SI::IDBKeyRange as u64){
                let state=self.state.borrow();let range=match &state.storage.as_ref().unwrap().objects[handle.slot as usize]{StorageObject::Range(range)=>range.clone(),_=>unreachable!()};(None,Some(range))
            }else{let key=self.storage_to_key(value)?;(Some(key.clone()),Some(KeyRange::only(key)))}
        }else{(None,None)};
        let value=if matches!(member,"add"|"put"){Some(self.storage_clone_value(self.storage_arg(args,0)?,&mut Vec::new())?)}else{None};
        let limit=if matches!(member,"getAll"|"getAllKeys") {args.get(1).filter(|v|!matches!(v,JsValue::Undefined)).map(|v|self.js.binding_to_number(v).map(|n|n.max(0.0) as usize)).transpose()?}else{None};
        let outcome={let mut state=self.state.borrow_mut();let storage=state.storage.as_mut().unwrap();
            let StorageObject::Transaction{value:Some(tx),..}=&mut storage.objects[transaction]else{return Err(Throw::type_error("Inactive transaction"))};
            match member {
                "add"|"put"=>tx.store_mut(&name).and_then(|store|store.put(value.unwrap(),key,member=="put")).map(StoreResult::Key),
                "get"|"getKey"|"getAll"|"getAllKeys"|"count"=>tx.store(&name).map(|store|{
                    let records=store.records(range.as_ref(),limit);
                    match member {"get"=>StoreResult::Value(records.first().map(|(_,value)|value.clone())),
                        "getKey"=>StoreResult::OptionalKey(records.first().map(|(key,_)|key.clone())),
                        "getAll"=>StoreResult::Values(records.into_iter().map(|(_,value)|value).collect()),
                        "getAllKeys"=>StoreResult::Keys(records.into_iter().map(|(key,_)|key).collect()),
                        _=>StoreResult::Count(store.count(range.as_ref()))}
                }),
                "delete"=>tx.store_mut(&name).map(|store|{if let Some(key)=key.as_ref(){store.delete(key);}StoreResult::None}),
                "clear"=>tx.store_mut(&name).map(|store|{store.clear();StoreResult::None}),
                _=>Err(idb::Error::InvalidState),
            }
        };
        let (request,wrapper)=self.storage_request(false,this,self.storage_wrapper(transaction)?)?;
        let result=match outcome {
            Ok(StoreResult::None)=>JsValue::Undefined,Ok(StoreResult::Count(count))=>JsValue::num(count as f64),
            Ok(StoreResult::Key(key))=>self.storage_from_key(&key)?,
            Ok(StoreResult::OptionalKey(key))=>key.as_ref().map_or(Ok(JsValue::Undefined),|key|self.storage_from_key(key))?,
            Ok(StoreResult::Keys(keys))=>{let mut items=Vec::new();for key in keys{items.push(self.storage_from_key(&key)?)}self.js.binding_create_array(items)?},
            Ok(StoreResult::Value(value))=>value.as_ref().map_or(Ok(JsValue::Undefined),|value|self.storage_from_value(value))?,
            Ok(StoreResult::Values(values))=>{let mut items=Vec::new();for value in values{items.push(self.storage_from_value(&value)?)}self.js.binding_create_array(items)?},
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
            StorageObject::Index{transaction,store,name}=>(*transaction,store.clone(),name.clone()),_=>unreachable!()}};
        if member=="name" {return Ok(JsValue::str(index_name))}
        if member=="objectStore" {return self.storage_allocate(SI::IDBObjectStore,StorageObject::Store{transaction,name:store_name})}
        if matches!(member,"keyPath"|"unique"|"multiEntry") {
            let (path,unique,multi)={let state=self.state.borrow();let StorageObject::Transaction{value:Some(tx),..}=&state.storage.as_ref().unwrap().objects[transaction]else{return Err(Throw::type_error("Inactive transaction"))};
                let index=tx.store(&store_name).and_then(|store|store.index(&index_name)).map_err(|error|Throw::type_error(format!("{error:?}")))?;
                (index.key_path.clone(),index.unique,index.multi_entry)};
            return match member {"keyPath"=>self.storage_key_path_value(&path),
                "unique"=>Ok(JsValue::Bool(unique)),_=>Ok(JsValue::Bool(multi))}
        }
        let range=self.storage_query_range(args.first())?;
        self.storage_transaction_accepts_request(transaction)?;
        let limit=if matches!(member,"getAll"|"getAllKeys") {args.get(1).filter(|v|!matches!(v,JsValue::Undefined)).map(|v|self.js.binding_to_number(v).map(|n|n.max(0.0) as usize)).transpose()?}else{None};
        if member=="openCursor"||member=="openKeyCursor" {
            let direction=self.storage_direction(args.get(1))?;
            let cursor={let state=self.state.borrow();let StorageObject::Transaction{value:Some(tx),..}=&state.storage.as_ref().unwrap().objects[transaction]else{return Err(Throw::type_error("Inactive transaction"))};
                let store=tx.store(&store_name).map_err(|error|Throw::type_error(format!("{error:?}")))?;
                store.index(&index_name).map_err(|error|Throw::type_error(format!("{error:?}")))?.cursor(store,range.as_ref(),direction)};
            return self.storage_open_cursor(cursor,transaction,slot,this,member=="openCursor");
        }
        let outcome={let state=self.state.borrow();let StorageObject::Transaction{value:Some(tx),..}=&state.storage.as_ref().unwrap().objects[transaction]else{return Err(Throw::type_error("Inactive transaction"))};
            let store=tx.store(&store_name).map_err(|error|Throw::type_error(format!("{error:?}")))?;
            let index=store.index(&index_name).map_err(|error|Throw::type_error(format!("{error:?}")))?;
            let records=index.records(store,range.as_ref(),limit);
            match member {"get"=>StoreResult::Value(records.first().map(|(_,_,value)|value.clone())),
                "getKey"=>StoreResult::OptionalKey(records.first().map(|(_,key,_)|key.clone())),
                "getAll"=>StoreResult::Values(records.into_iter().map(|(_,_,value)|value).collect()),
                "getAllKeys"=>StoreResult::Keys(records.into_iter().map(|(_,key,_)|key).collect()),
                "count"=>StoreResult::Count(index.count(range.as_ref())),
                _=>return Err(Throw::type_error("Unknown IDBIndex member")),
            }};
        let (request,wrapper)=self.storage_request(false,this,self.storage_wrapper(transaction)?)?;
        let result=match outcome {StoreResult::Value(value)=>value.as_ref().map_or(Ok(JsValue::Undefined),|value|self.storage_from_value(value))?,
            StoreResult::OptionalKey(key)=>key.as_ref().map_or(Ok(JsValue::Undefined),|key|self.storage_from_key(key))?,
            StoreResult::Keys(keys)=>{let mut items=Vec::new();for key in keys{items.push(self.storage_from_key(&key)?)}self.js.binding_create_array(items)?},
            StoreResult::Values(values)=>{let mut items=Vec::new();for value in values{items.push(self.storage_from_value(&value)?)}self.js.binding_create_array(items)?},
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
                let value=if member=="update"{Some(self.storage_clone_value(self.storage_arg(args,0)?,&mut Vec::new())?)}else{None};
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
                StorageJob::OpenUpgrade{request,connection,name,old,new}=>{
                    let database=self.storage_allocate(SI::IDBDatabase,StorageObject::Database{connection,name,handlers:HashMap::new(),listeners:Vec::new()})?;
                    let (txid,tx)={let state=self.state.borrow();state.storage.as_ref().unwrap().idb.borrow_mut().begin(connection,Vec::<String>::new(),Mode::VersionChange,Durability::Default)}
                        .map_err(|error|self.storage_idb_error(error))?;
                    let transaction=self.storage_allocate(SI::IDBTransaction,StorageObject::Transaction{connection,id:txid,value:Some(tx),mode:Mode::VersionChange,scope:Vec::new(),queued:false,active_for_requests:true,handlers:HashMap::new(),listeners:Vec::new()})?;
                    let transaction_slot=self.storage_slot(&transaction,SI::IDBTransaction)?;
                    if let Some(StorageObject::Request{result,transaction:stored,ready,error,..})=self.state.borrow_mut().storage.as_mut().unwrap().objects.get_mut(request){
                        *result=database.clone();*stored=transaction;*ready=true;*error=JsValue::Null;
                    }
                    self.storage_dispatch(request,"upgradeneeded",Some(old),Some(new))?;
                    self.storage_schedule(StorageJob::TransactionComplete{transaction:transaction_slot})?;
                    self.storage_schedule(StorageJob::RequestSuccess{request,result:database})?;
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
                        let result=self.state.borrow().storage.as_ref().unwrap().idb.borrow_mut().finish(connection,id,value,true);
                        match result {Ok(())=>self.storage_dispatch(transaction,"complete",None,None)?,Err(_)=>self.storage_dispatch(transaction,"abort",None,None)?}
                    }
                },
                StorageJob::StorageEvent(_)=>{},
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
        match value {
            JsValue::Num(number)=>Key::number(*number).map_err(|error|self.storage_idb_error(error)),
            JsValue::Str(string)=>Ok(Key::String(string.to_utf8_lossy())),
            JsValue::Object(object) if matches!(object.borrow().get_own("[[DateValue]]"),Some(JsValue::Num(_)))=>{
                let Some(JsValue::Num(value))=object.borrow().get_own("[[DateValue]]") else{unreachable!()};
                Key::date(value).map_err(|error|self.storage_idb_error(error))
            },
            JsValue::Object(object) if matches!(object.borrow().exotic,Exotic::Array)=>{
                let length=self.js.binding_get_property(value,"length")?;
                let count=self.js.binding_to_number(&length)? as usize;
                if count>100_000{return Err(self.storage_dom_error("DataError","Key is too long"))}
                let mut items=Vec::new();for index in 0..count {let item=self.js.binding_get_property(value,&index.to_string())?;items.push(self.storage_to_key(&item)?)}
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
    fn storage_clone_value(&mut self,value:&JsValue,seen:&mut Vec<usize>)->Result<StructuredValue,Throw>{
        match value {
            JsValue::Undefined=>Ok(StructuredValue::Undefined),
            JsValue::Null=>Ok(StructuredValue::Null),
            JsValue::Bool(value)=>Ok(StructuredValue::Bool(*value)),JsValue::Num(value)=>Ok(StructuredValue::Number(*value)),
            JsValue::Str(value)=>Ok(StructuredValue::String(value.to_utf8_lossy())),
            JsValue::Object(object) if matches!(object.borrow().get_own("[[DateValue]]"),Some(JsValue::Num(_)))=>{
                let Some(JsValue::Num(value))=object.borrow().get_own("[[DateValue]]") else{unreachable!()};Ok(StructuredValue::Date(value))
            },
            JsValue::Object(_) if self.storage_binary_bytes(value)?.is_some()=>Ok(StructuredValue::Binary(self.storage_binary_bytes(value)?.unwrap())),
            JsValue::Object(object)=>{
                let pointer=Rc::as_ptr(&object.0) as usize;
                if object.borrow().func.is_some()||!matches!(object.borrow().exotic,Exotic::Ordinary|Exotic::Array)||seen.contains(&pointer){
                    return Err(self.storage_dom_error("DataCloneError","Value cannot be cloned"));
                }
                seen.push(pointer);
                let result=if matches!(object.borrow().exotic,Exotic::Array) {
                    let length=self.js.binding_get_property(value,"length")?;let count=self.js.binding_to_number(&length)? as usize;
                    if count>100_000{return Err(self.storage_dom_error("DataCloneError","Array is too long"))}
                    let mut items=Vec::new();for index in 0..count {let item=self.js.binding_get_property(value,&index.to_string())?;items.push(self.storage_clone_value(&item,seen)?)}
                    StructuredValue::Array(items)
                }else{
                    let names=self.js.binding_own_property_keys(value)?;let mut map=BTreeMap::new();
                    for name in names {let item=self.js.binding_get_property(value,&name)?;map.insert(name,self.storage_clone_value(&item,seen)?);}
                    StructuredValue::Object(map)
                };seen.pop();Ok(result)
            },
            _=>Err(self.storage_dom_error("DataCloneError","Value cannot be cloned")),
        }
    }
    fn storage_from_value(&mut self,value:&StructuredValue)->Result<JsValue,Throw>{
        match value {StructuredValue::Undefined=>Ok(JsValue::Undefined),StructuredValue::Null=>Ok(JsValue::Null),StructuredValue::Bool(value)=>Ok(JsValue::Bool(*value)),
            StructuredValue::Number(value)=>Ok(JsValue::num(*value)),
            StructuredValue::Date(value)=>{let global=self.js.binding_global_object();let constructor=self.js.binding_get_property(&global,"Date")?;self.js.binding_construct(&constructor,vec![JsValue::num(*value)])},
            StructuredValue::String(value)=>Ok(JsValue::str(value)),StructuredValue::Binary(value)=>self.js.binding_create_array_buffer(value.clone()),
            StructuredValue::Array(items)=>{let mut values=Vec::new();for item in items{values.push(self.storage_from_value(item)?)}self.js.binding_create_array(values)},
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
}
