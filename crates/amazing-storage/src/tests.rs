use crate::{
    buckets::*,
    cache::*,
    core::*,
    idb::{self, *},
    web::*,
};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

fn partition() -> StorageKey {
    StorageKey::new("https://a.example", "https://top.example")
}

#[test]
fn profile_quota_counts_web_idb_and_cache_together_and_persists_grant() {
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("amazing-profile-quota-{}-{nonce}", std::process::id()));
    let quota = Arc::new(Mutex::new(QuotaLedger::new(30)));
    let mut stores = Vec::new();
    for name in ["web", "idb", "cache"] {
        let mut disk = DiskStore::open(root.join(name), 1000).unwrap();
        disk.attach_quota(quota.clone());
        stores.push(disk);
    }
    let entry = |namespace: &str| EntryKey { bucket: bucket(), namespace: namespace.into(), key: b"a".to_vec() };
    stores[0].put(entry("w"), vec![1; 10]).unwrap();
    stores[1].put(entry("i"), vec![2; 10]).unwrap();
    assert_eq!(stores[2].put(entry("c"), vec![3; 10]), Err(crate::core::Error::QuotaExceeded));
    assert_eq!(quota.lock().unwrap().usage(&partition()), 24);
    let mut manager = crate::manager::StorageManager::open(
        DiskStore::open(root.join("manager"), 1000).unwrap(), quota.clone()).unwrap();
    assert_eq!(manager.estimate(&partition()), crate::manager::Estimate { usage: 24, quota: 30 });
    assert!(!manager.persisted(&partition()));
    assert!(manager.persist(partition()).unwrap());
    drop(manager);
    let manager = crate::manager::StorageManager::open(
        DiskStore::open(root.join("manager"), 1000).unwrap(), quota.clone()).unwrap();
    assert!(manager.persisted(&partition()));
    assert!(stores[0].delete(&entry("w")).unwrap());
    stores[2].put(entry("c"), vec![3; 10]).unwrap();
    assert_eq!(manager.estimate(&partition()).usage, 24);
    drop(manager);
    drop(stores);
    let reopened_quota = Arc::new(Mutex::new(QuotaLedger::new(30)));
    let mut reopened = Vec::new();
    for name in ["web", "idb", "cache"] {
        let mut disk = DiskStore::open(root.join(name), 1000).unwrap();
        disk.attach_quota(reopened_quota.clone());
        reopened.push(disk);
    }
    let manager = crate::manager::StorageManager::open(
        DiskStore::open(root.join("manager"), 1000).unwrap(), reopened_quota).unwrap();
    assert_eq!(manager.estimate(&partition()).usage, 24);
    assert!(manager.persisted(&partition()));
    drop(manager);
    drop(reopened);
    let _ = std::fs::remove_dir_all(root);
}
fn bucket() -> BucketKey {
    BucketKey::new(partition(), "default")
}
#[test]
fn partition_quota_eviction_and_rollback() {
    let mut store = MemoryStore::new(12);
    let a = EntryKey {
        bucket: bucket(),
        namespace: "n".into(),
        key: b"a".to_vec(),
    };
    store.put(a.clone(), vec![1; 8]).unwrap();
    assert_eq!(store.usage(&partition()), 10);
    assert_eq!(
        store.put(
            EntryKey {
                key: b"b".to_vec(),
                ..a.clone()
            },
            vec![1; 8]
        ),
        Err(crate::core::Error::QuotaExceeded)
    );
    assert_eq!(store.get(&a), Some(vec![1; 8]));
    let other = BucketKey::new(partition(), "other");
    assert!(store
        .evict_to(&partition(), 0, &[other])
        .contains(&bucket()));
    assert_eq!(store.usage(&partition()), 0);
}
#[test]
fn disk_log_survives_every_truncated_tail() {
    let root = std::env::temp_dir().join(format!("amazing-store-recovery-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let key = EntryKey {
        bucket: bucket(),
        namespace: "n".into(),
        key: b"key".to_vec(),
    };
    let data = {
        let mut store = DiskStore::open(&root, 10000).unwrap();
        store.put(key.clone(), b"first".to_vec()).unwrap();
        store.put(key.clone(), b"second".to_vec()).unwrap();
        std::fs::read(root.join("log")).unwrap()
    };
    let first_len = u32::from_le_bytes(data[8..12].try_into().unwrap()) as usize;
    let first_end = 8 + 4 + first_len + 4;
    for len in 0..=data.len() {
        let path = root.join(format!("case-{len}"));
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("log"), &data[..len]).unwrap();
        let mut reopened = DiskStore::open(&path, 10000).unwrap();
        let value = reopened.get(&key);
        let expected = if len == data.len() {
            Some(b"second".to_vec())
        } else if len >= first_end {
            Some(b"first".to_vec())
        } else {
            None
        };
        assert_eq!(value, expected, "offset {len}");
        reopened.put(key.clone(), b"after".to_vec()).unwrap();
        assert_eq!(
            DiskStore::open(&path, 10000).unwrap().get(&key),
            Some(b"after".to_vec())
        );
    }
    let mut store = DiskStore::open(&root, 10000).unwrap();
    store.checkpoint().unwrap();
    store.checkpoint().unwrap();
    store.checkpoint().unwrap();
    assert_eq!(
        DiskStore::open(&root, 10000).unwrap().get(&key),
        Some(b"second".to_vec())
    );
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn disk_store_quarantines_corrupt_checkpoint_and_starts_fresh() {
    // IA-REAL-2: a checkpoint file that exists but fails to parse used to make
    // `DiskStore::open` return `Error::Corrupt` outright. A browser must never
    // fail to open storage (and so never fail to load a page) because its
    // on-disk store is damaged: quarantine it aside and start fresh instead.
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("amazing-store-quarantine-corrupt-{}-{nonce}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("checkpoint.0"), b"not a real checkpoint").unwrap();
    let key = EntryKey { bucket: bucket(), namespace: "n".into(), key: b"key".to_vec() };
    let mut store = DiskStore::open(&root, 10000).unwrap();
    assert_eq!(store.get(&key), None);
    store.put(key.clone(), b"value".to_vec()).unwrap();
    assert_eq!(store.get(&key), Some(b"value".to_vec()));
    drop(store);
    let parent = root.parent().unwrap();
    let root_name = root.file_name().unwrap().to_str().unwrap().to_string();
    let quarantined: Vec<_> = std::fs::read_dir(parent).unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(&format!("{root_name}.corrupt-")))
        .collect();
    assert_eq!(quarantined.len(), 1, "expected exactly one quarantined copy, found {quarantined:?}");
    // Re-opening the (now fresh) path keeps the post-quarantine data.
    assert_eq!(DiskStore::open(&root, 10000).unwrap().get(&key), Some(b"value".to_vec()));
    std::fs::remove_dir_all(&root).unwrap();
    for name in quarantined {
        let _ = std::fs::remove_dir_all(parent.join(name));
    }
}
#[test]
fn disk_store_quarantines_unrecognized_format_version_and_starts_fresh() {
    // IA-REAL-2: an on-disk store stamped with a format version this build
    // does not recognize (e.g. written by a future or old build) must be
    // treated the same as corrupt -- quarantined, never returned as an error
    // that fails page load.
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("amazing-store-quarantine-version-{}-{nonce}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let mut bogus = Vec::new();
    bogus.extend_from_slice(b"AMSTFMT1");
    bogus.extend_from_slice(&99u32.to_le_bytes());
    std::fs::write(root.join("format"), &bogus).unwrap();
    let key = EntryKey { bucket: bucket(), namespace: "n".into(), key: b"key".to_vec() };
    let mut store = DiskStore::open(&root, 10000).unwrap();
    assert_eq!(store.get(&key), None);
    store.put(key.clone(), b"value".to_vec()).unwrap();
    assert_eq!(store.get(&key), Some(b"value".to_vec()));
    drop(store);
    let parent = root.parent().unwrap();
    let root_name = root.file_name().unwrap().to_str().unwrap().to_string();
    let quarantined: Vec<_> = std::fs::read_dir(parent).unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(&format!("{root_name}.corrupt-")))
        .collect();
    assert_eq!(quarantined.len(), 1, "expected exactly one quarantined copy, found {quarantined:?}");
    std::fs::remove_dir_all(&root).unwrap();
    for name in quarantined {
        let _ = std::fs::remove_dir_all(parent.join(name));
    }
}
#[test]
fn web_storage_order_events_and_sessions() {
    let mut storage = WebStorage::new(100);
    let area = storage.local(bucket());
    assert!(area.set(vec![1], vec![2], 7).unwrap().is_some());
    assert!(area.set(vec![3], vec![4], 7).unwrap().is_some());
    assert!(area.set(vec![1], vec![5], 7).unwrap().is_some());
    assert_eq!(area.key(0), Some(&[1][..]));
    assert!(area.set(vec![1], vec![5], 7).unwrap().is_none());
    assert_eq!(area.remove(&[1], 7).unwrap().old_value, Some(vec![5]));
    assert_eq!(area.key(0), Some(&[3][..]));
    let decoded = StorageArea::decode(&area.encode(), 100).unwrap();
    assert_eq!(decoded.get(&[3]), Some(&[4][..]));
    storage
        .session(1, partition())
        .set(vec![1], vec![2], 1)
        .unwrap();
    storage.clone_session(1, 2);
    storage.close_session(1);
    assert_eq!(storage.session(2, partition()).get(&[1]), Some(&[2][..]));
}
#[test]
fn cache_vary_and_options() {
    let mut cache = Cache::default();
    let mut headers = BTreeMap::new();
    headers.insert("Accept-Language".into(), "en".into());
    let req = Request {
        url: "https://a.test/x?q=1".into(),
        method: "GET".into(),
        headers: headers.clone(),
    };
    let mut response_headers = BTreeMap::new();
    response_headers.insert("Vary".into(), "accept-language".into());
    cache
        .put(
            req.clone(),
            Response {
                status: 200,
                headers: response_headers,
                body: b"ok".to_vec(),
            },
        )
        .unwrap();
    let mut other = req.clone();
    other.url = "https://a.test/x?q=2".into();
    other.headers.insert("Accept-Language".into(), "fr".into());
    assert!(cache.match_one(&other, MatchOptions::default()).is_none());
    assert!(cache
        .match_one(
            &other,
            MatchOptions {
                ignore_search: true,
                ignore_vary: true,
                ..Default::default()
            }
        )
        .is_some());
    assert_eq!(cache.delete(&req, MatchOptions::default()), 1);
}
#[test]
fn idb_keys_ranges_indexes_and_cursor() {
    assert!(Key::number(f64::NAN).is_err());
    assert_eq!(Key::number(-0.0).unwrap(), Key::number(0.0).unwrap());
    let range = KeyRange::bound(
        Key::number(1.0).unwrap(),
        Key::number(3.0).unwrap(),
        true,
        false,
    )
    .unwrap();
    assert!(!range.includes(&Key::number(1.0).unwrap()));
    assert!(range.includes(&Key::number(3.0).unwrap()));
    let mut store = ObjectStore::new(Some(KeyPath::Single("id".into())), true).unwrap();
    store
        .create_index("name", KeyPath::Single("name".into()), true, false)
        .unwrap();
    let make = |name: &str| {
        StructuredValue::Object(BTreeMap::from([(
            "name".into(),
            StructuredValue::String(name.into()),
        )]))
    };
    assert_eq!(
        store.put(make("one"), None, false).unwrap(),
        Key::number(1.0).unwrap()
    );
    assert_eq!(
        store.put(make("two"), None, false).unwrap(),
        Key::number(2.0).unwrap()
    );
    assert_eq!(
        store.put(make("one"), None, false),
        Err(idb::Error::Constraint)
    );
    assert_eq!(store.count(None), 2);
    assert!(store
        .index("name")
        .unwrap()
        .get(&store, &Key::String("two".into()))
        .is_some());
    let mut cursor = store.cursor(None, Direction::Next);
    assert_eq!(
        cursor.advance(1).unwrap().unwrap().key,
        Key::number(1.0).unwrap()
    );
    assert_eq!(
        cursor
            .continue_to(Some(&Key::number(2.0).unwrap()))
            .unwrap()
            .unwrap()
            .key,
        Key::number(2.0).unwrap()
    );
    assert!(cursor.advance(1).unwrap().is_none());
}
#[test]
fn idb_versionchange_block_and_transaction_rollback() {
    let mut engine = IndexedDb::default();
    let key = bucket();
    let opened = engine.open(key.clone(), "db", Some(1)).unwrap();
    let id = opened.connection.unwrap();
    assert!(matches!(
        opened.events[0],
        OpenEvent::UpgradeNeeded { old: 0, new: 1 }
    ));
    let blocked = engine.open(key.clone(), "db", Some(2)).unwrap();
    assert!(blocked.blocked);
    engine.close(id).unwrap();
    let resumed = engine.resume_pending();
    assert_eq!(resumed.len(), 1);
    let id = resumed[0].connection.unwrap();
    let (txid, mut tx) = engine
        .begin(id, [], Mode::VersionChange, Durability::Strict)
        .unwrap();
    tx.database_mut()
        .unwrap()
        .create_store("s", None, false)
        .unwrap();
    engine.finish(id, txid, tx, true).unwrap();
    assert!(matches!(
        engine.take_events().as_slice(),
        [OpenEvent::Success { version: 2, .. }]
    ));
    let (txid, mut tx) = engine
        .begin(id, ["s".into()], Mode::ReadWrite, Durability::Strict)
        .unwrap();
    tx.store_mut("s")
        .unwrap()
        .put(
            StructuredValue::String("x".into()),
            Some(Key::String("a".into())),
            false,
        )
        .unwrap();
    tx.abort();
    assert_eq!(engine.finish(id, txid, tx, true), Err(idb::Error::Abort));
    assert_eq!(
        engine
            .database(&key, "db")
            .unwrap()
            .store("s")
            .unwrap()
            .count(None),
        0
    );
}
#[test]
fn idb_index_cursor_primary_key_and_transaction_scheduling() {
    let mut engine = IndexedDb::default();
    let key = bucket();
    let id = engine
        .open(key.clone(), "db", Some(1))
        .unwrap()
        .connection
        .unwrap();
    let (txid, mut tx) = engine
        .begin(id, [], Mode::VersionChange, Durability::Strict)
        .unwrap();
    tx.database_mut()
        .unwrap()
        .create_store("s", None, false)
        .unwrap();
    engine.finish(id, txid, tx, true).unwrap();
    let (write_id, mut write) = engine
        .begin(id, ["s".into()], Mode::ReadWrite, Durability::Strict)
        .unwrap();
    assert!(matches!(
        engine.begin(id, ["s".into()], Mode::ReadOnly, Durability::Default),
        Err(idb::Error::Blocked)
    ));
    let store = write.store_mut("s").unwrap();
    store
        .create_index("i", KeyPath::Single("group".into()), false, false)
        .unwrap();
    for pk in ["a", "b"] {
        let value = StructuredValue::Object(BTreeMap::from([(
            "group".into(),
            StructuredValue::String("x".into()),
        )]));
        store
            .put(value, Some(Key::String(pk.into())), false)
            .unwrap();
    }
    engine.finish(id, write_id, write, true).unwrap();
    let store = engine.database(&key, "db").unwrap().store("s").unwrap();
    let mut cursor = store
        .index("i")
        .unwrap()
        .cursor(store, None, Direction::Next);
    assert_eq!(
        cursor.advance(1).unwrap().unwrap().primary_key,
        Key::String("a".into())
    );
    assert_eq!(
        cursor
            .continue_primary_key(&Key::String("x".into()), &Key::String("b".into()))
            .unwrap()
            .unwrap()
            .primary_key,
        Key::String("b".into())
    );
    assert!(matches!(
        store
            .cursor(None, Direction::Next)
            .continue_primary_key(&Key::String("a".into()), &Key::String("a".into())),
        Err(idb::Error::InvalidState)
    ));
}
#[test]
fn idb_multientry_skips_invalid_array_members() {
    let mut store = ObjectStore::new(None, false).unwrap();
    store
        .create_index("tags", KeyPath::Single("tags".into()), true, true)
        .unwrap();
    let value = StructuredValue::Object(BTreeMap::from([(
        "tags".into(),
        StructuredValue::Array(vec![
            StructuredValue::String("a".into()),
            StructuredValue::Null,
            StructuredValue::String("a".into()),
        ]),
    )]));
    store
        .put(value, Some(Key::number(1.0).unwrap()), false)
        .unwrap();
    assert_eq!(store.index("tags").unwrap().count(None), 1);
    let duplicate = StructuredValue::Object(BTreeMap::from([(
        "tags".into(),
        StructuredValue::Array(vec![StructuredValue::String("a".into())]),
    )]));
    assert_eq!(
        store.put(duplicate, Some(Key::number(2.0).unwrap()), false),
        Err(idb::Error::Constraint)
    );
}
#[test]
fn durable_web_cache_and_idb_reopen() {
    let root = std::env::temp_dir().join(format!("amazing-storage-engines-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    {
        let disk = DiskStore::open(root.join("web"), 100_000).unwrap();
        let mut web = PersistentWebStorage::open(disk, 10_000).unwrap();
        web.change_local(bucket(), |a| a.set(vec![0xd800], vec![10], 1))
            .unwrap();
        web.checkpoint().unwrap();
    }
    {
        let disk = DiskStore::open(root.join("web"), 100_000).unwrap();
        let web = PersistentWebStorage::open(disk, 10_000).unwrap();
        assert_eq!(
            web.local(&bucket()).unwrap().get(&[0xd800]),
            Some(&[10][..])
        );
    }
    {
        let disk = DiskStore::open(root.join("cache"), 100_000).unwrap();
        let mut caches = PersistentCacheStorage::open(disk).unwrap();
        caches
            .change(bucket(), "v1", |cache| {
                cache.put(
                    Request {
                        url: "https://a.test".into(),
                        method: "GET".into(),
                        headers: BTreeMap::new(),
                    },
                    Response {
                        status: 200,
                        headers: BTreeMap::new(),
                        body: vec![0, 255],
                    },
                )
            })
            .unwrap();
    }
    {
        let disk = DiskStore::open(root.join("cache"), 100_000).unwrap();
        let caches = PersistentCacheStorage::open(disk).unwrap();
        assert_eq!(caches.get(&bucket(), "v1").unwrap().keys().len(), 1);
    }
    {
        let disk = DiskStore::open(root.join("idb"), 100_000).unwrap();
        let mut engine = IndexedDb::with_disk(disk).unwrap();
        let id = engine
            .open(bucket(), "db", Some(1))
            .unwrap()
            .connection
            .unwrap();
        let (txid, mut tx) = engine
            .begin(id, [], Mode::VersionChange, Durability::Strict)
            .unwrap();
        tx.database_mut()
            .unwrap()
            .create_store("s", None, false)
            .unwrap();
        engine.finish(id, txid, tx, true).unwrap();
        let (txid, mut tx) = engine
            .begin(id, ["s".into()], Mode::ReadWrite, Durability::Strict)
            .unwrap();
        tx.store_mut("s")
            .unwrap()
            .put(
                StructuredValue::Binary(vec![1, 2, 3]),
                Some(Key::String("k".into())),
                false,
            )
            .unwrap();
        engine.finish(id, txid, tx, true).unwrap();
    }
    {
        let disk = DiskStore::open(root.join("idb"), 100_000).unwrap();
        let engine = IndexedDb::with_disk(disk).unwrap();
        assert_eq!(
            engine
                .database(&bucket(), "db")
                .unwrap()
                .store("s")
                .unwrap()
                .get(&Key::String("k".into())),
            Some(&StructuredValue::Binary(vec![1, 2, 3]))
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn bucket_names_follow_the_lowercase_charset_and_length() {
    for good in [
        "abcdefghijklmnopqrstuvwxyz0123456789-_",
        "2021-01-01",
        "2021_01_01",
        "2021_01_01_",
        "2021-01-01-",
        "a",
        "0",
    ] {
        assert!(valid_bucket_name(good), "{good:?} should be a valid name");
    }
    for bad in [
        "_bucket",
        "-bucket",
        "bucket name",
        "bUcKet123",
        "bucket♦",
        "2021/01/01",
        "   ",
        "",
        "mjnkhtwsiyjsrxvrzzqafldfvomqopdjfiuxqelfkllcugrhvvblkvmiqlguhhqepoggyu",
    ] {
        assert!(!valid_bucket_name(bad), "{bad:?} should be rejected");
    }
    // 64 characters is the inclusive limit.
    let limit = "a".repeat(MAX_BUCKET_NAME);
    assert!(valid_bucket_name(&limit));
    assert!(!valid_bucket_name(&format!("{limit}a")));
}

#[test]
fn bucket_registry_persists_policy_and_invalidates_deleted_handles() {
    let root = std::env::temp_dir().join(format!("amazing-buckets-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let key = partition();
    let strict = Policy { durability: Durability::Strict, quota: Some(4096), ..Policy::default() };
    let mut registry =
        BucketRegistry::load(DiskStore::open(root.join("buckets"), 100_000).unwrap()).unwrap();
    let epoch = registry.open(key.clone(), "one".into(), strict.clone()).unwrap();
    registry.open(key.clone(), "two".into(), Policy::default()).unwrap();
    assert_eq!(registry.keys(&key), vec!["one".to_owned(), "two".to_owned()]);
    // The existing policy is kept when the bucket already exists.
    let same = registry.open(key.clone(), "one".into(), Policy::default()).unwrap();
    assert_eq!(same, epoch);
    assert_eq!(registry.policy(&key, "one"), Some(&strict));
    assert!(registry.handle_valid(&key, "one", epoch));
    assert!(!registry.handle_valid(&key, "one", epoch + 1));
    assert!(registry.delete(&key, "one").unwrap());
    assert!(!registry.delete(&key, "one").unwrap());
    assert!(!registry.handle_valid(&key, "one", epoch));
    // Recreating the name yields a new epoch, invalidating the old handle.
    let remade = registry.open(key.clone(), "one".into(), Policy::default()).unwrap();
    assert_ne!(remade, epoch);
    assert!(registry.handle_valid(&key, "one", remade));

    registry
        .update(&key, "two", Policy { persisted: true, expires: Some(42), ..Policy::default() })
        .unwrap();
    drop(registry);
    let registry =
        BucketRegistry::load(DiskStore::open(root.join("buckets"), 100_000).unwrap()).unwrap();
    assert_eq!(registry.keys(&key), vec!["one".to_owned(), "two".to_owned()]);
    assert_eq!(
        registry.policy(&key, "two"),
        Some(&Policy { persisted: true, expires: Some(42), ..Policy::default() })
    );
    std::fs::remove_dir_all(root).unwrap();
}
