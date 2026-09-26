//! WebIDL surface metadata for Storage, IndexedDB, and Cache API.
//!
//! The driver owns opaque JS handles and implements these operations against
//! amazing-storage. This module has no JS-runtime dependency.

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum StorageInterface {
    Storage,
    IDBFactory,
    IDBRequest,
    IDBOpenDBRequest,
    IDBDatabase,
    IDBTransaction,
    IDBObjectStore,
    IDBIndex,
    IDBCursor,
    IDBCursorWithValue,
    IDBKeyRange,
    IDBVersionChangeEvent,
    CacheStorage,
    Cache,
    StorageManager,
    StorageBucketManager,
    StorageBucket,
    LockManager,
    Lock,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemberKind { Method, Getter, Setter, StaticMethod }

#[derive(Clone, Copy, Debug)]
pub struct Member {
    pub name: &'static str,
    pub kind: MemberKind,
    pub length: u8,
}

#[derive(Clone, Copy, Debug)]
pub struct Interface {
    pub kind: StorageInterface,
    pub name: &'static str,
    pub parent: Option<StorageInterface>,
    pub members: &'static [Member],
}

const fn method(name: &'static str, length: u8) -> Member { Member { name, kind: MemberKind::Method, length } }
const fn getter(name: &'static str) -> Member { Member { name, kind: MemberKind::Getter, length: 0 } }
const fn setter(name: &'static str) -> Member { Member { name, kind: MemberKind::Setter, length: 0 } }
const fn static_method(name: &'static str, length: u8) -> Member { Member { name, kind: MemberKind::StaticMethod, length } }

pub const INTERFACES: &[Interface] = &[
    Interface { kind: StorageInterface::IDBVersionChangeEvent, name: "IDBVersionChangeEvent", parent: None, members: &[] },
    Interface { kind: StorageInterface::Storage, name: "Storage", parent: None, members: &[
        getter("length"), method("key", 1), method("getItem", 1), method("setItem", 2),
        method("removeItem", 1), method("clear", 0),
    ]},
    Interface { kind: StorageInterface::IDBFactory, name: "IDBFactory", parent: None, members: &[
        method("open", 1), method("deleteDatabase", 1), method("databases", 0), method("cmp", 2),
    ]},
    Interface { kind: StorageInterface::IDBRequest, name: "IDBRequest", parent: None, members: &[
        method("addEventListener", 2), method("removeEventListener", 2),
        getter("result"), getter("error"), getter("source"), getter("transaction"), getter("readyState"),
        getter("onsuccess"), setter("onsuccess"), getter("onerror"), setter("onerror"),
    ]},
    Interface { kind: StorageInterface::IDBOpenDBRequest, name: "IDBOpenDBRequest", parent: Some(StorageInterface::IDBRequest), members: &[
        getter("onblocked"), setter("onblocked"), getter("onupgradeneeded"), setter("onupgradeneeded"),
    ]},
    Interface { kind: StorageInterface::IDBDatabase, name: "IDBDatabase", parent: None, members: &[
        method("addEventListener", 2), method("removeEventListener", 2),
        getter("name"), getter("version"), getter("objectStoreNames"), getter("onversionchange"), setter("onversionchange"),
        getter("onclose"), setter("onclose"), getter("onerror"), setter("onerror"),
        method("createObjectStore", 1), method("deleteObjectStore", 1), method("transaction", 1), method("close", 0),
    ]},
    Interface { kind: StorageInterface::IDBTransaction, name: "IDBTransaction", parent: None, members: &[
        method("addEventListener", 2), method("removeEventListener", 2),
        getter("mode"), getter("durability"), getter("db"), getter("objectStoreNames"), getter("error"),
        getter("oncomplete"), setter("oncomplete"), getter("onabort"), setter("onabort"),
        getter("onerror"), setter("onerror"), method("objectStore", 1), method("abort", 0), method("commit", 0),
    ]},
    Interface { kind: StorageInterface::IDBObjectStore, name: "IDBObjectStore", parent: None, members: &[
        getter("name"), getter("keyPath"), getter("indexNames"), getter("transaction"), getter("autoIncrement"),
        method("add", 1), method("put", 1), method("get", 1), method("getAll", 0), method("getAllKeys", 0), method("getKey", 1),
        method("count", 0), method("delete", 1), method("clear", 0), method("openCursor", 0),
        method("openKeyCursor", 0), method("createIndex", 2), method("index", 1), method("deleteIndex", 1),
    ]},
    Interface { kind: StorageInterface::IDBIndex, name: "IDBIndex", parent: None, members: &[
        getter("name"), getter("objectStore"), getter("keyPath"), getter("multiEntry"), getter("unique"),
        method("get", 1), method("getAll", 0), method("getAllKeys", 0), method("getKey", 1), method("count", 0),
        method("openCursor", 0), method("openKeyCursor", 0),
    ]},
    Interface { kind: StorageInterface::IDBCursor, name: "IDBCursor", parent: None, members: &[
        getter("source"), getter("direction"), getter("key"), getter("primaryKey"), getter("request"),
        method("advance", 1), method("continue", 0), method("continuePrimaryKey", 2),
        method("update", 1), method("delete", 0),
    ]},
    Interface { kind: StorageInterface::IDBCursorWithValue, name: "IDBCursorWithValue", parent: Some(StorageInterface::IDBCursor), members: &[
        getter("value"),
    ]},
    Interface { kind: StorageInterface::IDBKeyRange, name: "IDBKeyRange", parent: None, members: &[
        getter("lower"), getter("upper"), getter("lowerOpen"), getter("upperOpen"), method("includes", 1),
        static_method("only", 1), static_method("lowerBound", 1), static_method("upperBound", 1), static_method("bound", 2),
    ]},
    Interface { kind: StorageInterface::CacheStorage, name: "CacheStorage", parent: None, members: &[
        method("match", 1), method("has", 1), method("open", 1), method("delete", 1), method("keys", 0),
    ]},
    Interface { kind: StorageInterface::Cache, name: "Cache", parent: None, members: &[
        method("match", 1), method("matchAll", 0), method("add", 1), method("addAll", 1),
        method("put", 2), method("delete", 1), method("keys", 0),
    ]},
    Interface { kind: StorageInterface::StorageManager, name: "StorageManager", parent: None, members: &[
        method("estimate", 0), method("persist", 0), method("persisted", 0),
    ]},
    Interface { kind: StorageInterface::StorageBucketManager, name: "StorageBucketManager", parent: None, members: &[
        method("open", 1), method("delete", 1), method("keys", 0),
    ]},
    Interface { kind: StorageInterface::StorageBucket, name: "StorageBucket", parent: None, members: &[
        getter("name"), getter("indexedDB"), getter("caches"),
        method("persist", 0), method("persisted", 0), method("estimate", 0),
        method("durability", 0), method("setExpires", 1), method("expires", 0),
    ]},
    Interface { kind: StorageInterface::LockManager, name: "LockManager", parent: None, members: &[
        method("request", 2), method("query", 0),
    ]},
    Interface { kind: StorageInterface::Lock, name: "Lock", parent: None, members: &[
        getter("name"), getter("mode"),
    ]},
];
