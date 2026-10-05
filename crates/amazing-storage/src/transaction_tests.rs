//! Durable transaction recovery controls, independent of browser bindings.
use crate::core::*;
use std::{
    fs,
    sync::{Arc, Mutex},
};

struct Profile(std::path::PathBuf);
impl Profile {
    fn new(label: &str) -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "storage-transactions-{label}-{}-{nonce}",
            std::process::id()
        )))
    }
    fn open(&self) -> DiskStore {
        DiskStore::open(&self.0, 10000).unwrap()
    }
}
impl Drop for Profile {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn key(partition: &str, name: &str) -> EntryKey {
    EntryKey {
        bucket: BucketKey::new(StorageKey::new(partition, "top"), "default"),
        namespace: "n".into(),
        key: name.as_bytes().to_vec(),
    }
}

#[test]
fn transaction_survives_every_torn_boundary_atomically_and_can_append() {
    let profile = Profile::new("boundaries");
    let a = key("one", "a");
    let b = key("one", "b");
    let c = key("two", "c");
    let mut disk = profile.open();
    disk.put(a.clone(), b"original".to_vec()).unwrap();
    disk.put(b.clone(), b"delete".to_vec()).unwrap();
    let prefix = fs::read(profile.0.join("log")).unwrap().len();
    disk.transaction(|next| {
        next.put(a.clone(), b"updated".to_vec())?;
        next.delete(&b);
        next.put(c.clone(), b"inserted".to_vec())?;
        assert_eq!(next.peek(&a), Some(b"updated".as_slice()));
        Ok(())
    })
    .unwrap();
    let log = fs::read(profile.0.join("log")).unwrap();
    drop(disk);
    for boundary in prefix..=log.len() {
        let case = profile.0.join(format!("cut-{boundary}"));
        fs::create_dir_all(&case).unwrap();
        fs::write(case.join("log"), &log[..boundary]).unwrap();
        let mut recovered = DiskStore::open(&case, 10000).unwrap();
        if boundary == log.len() {
            assert_eq!(recovered.get(&a), Some(b"updated".to_vec()));
            assert_eq!(recovered.get(&b), None);
            assert_eq!(recovered.get(&c), Some(b"inserted".to_vec()));
        } else {
            assert_eq!(
                recovered.get(&a),
                Some(b"original".to_vec()),
                "boundary {boundary}"
            );
            assert_eq!(recovered.get(&b), Some(b"delete".to_vec()));
            assert_eq!(recovered.get(&c), None);
        }
        recovered
            .transaction(|next| next.put(c.clone(), b"later".to_vec()))
            .unwrap();
        drop(recovered);
        assert_eq!(
            DiskStore::open(&case, 10000).unwrap().get(&c),
            Some(b"later".to_vec())
        );
    }
}

#[test]
fn rollback_errors_panics_noops_and_io_failure_leave_committed_state() {
    let profile = Profile::new("rollback");
    let a = key("one", "a");
    let mut disk = profile.open();
    disk.put(a.clone(), b"original".to_vec()).unwrap();
    let original = fs::read(profile.0.join("log")).unwrap();
    assert_eq!(
        disk.transaction(|next| {
            next.delete(&a);
            Err::<(), _>(Error::InvalidInput)
        }),
        Err(Error::InvalidInput)
    );
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _: Result<()> = disk.transaction(|next| {
            next.delete(&a);
            panic!("abort staged operation")
        });
    }))
    .is_err());
    disk.transaction(|next| {
        next.delete(&a);
        next.put(a.clone(), b"original".to_vec())
    })
    .unwrap();
    assert_eq!(fs::read(profile.0.join("log")).unwrap(), original);
    fs::rename(profile.0.join("log"), profile.0.join("saved-log")).unwrap();
    fs::create_dir(profile.0.join("log")).unwrap();
    assert!(matches!(
        disk.transaction(|next| next.put(a.clone(), b"failed".to_vec())),
        Err(Error::Io(_))
    ));
    assert_eq!(disk.get(&a), Some(b"original".to_vec()));
    fs::remove_dir(profile.0.join("log")).unwrap();
    fs::rename(profile.0.join("saved-log"), profile.0.join("log")).unwrap();
    disk.transaction(|next| next.put(a.clone(), b"committed".to_vec()))
        .unwrap();
    assert_eq!(profile.open().get(&a), Some(b"committed".to_vec()));
    let mut memory = MemoryStore::new(100);
    memory.put(a.clone(), b"original".to_vec()).unwrap();
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _: Result<()> = memory.transaction(|next| {
            next.delete(&a);
            panic!("abort")
        });
    }))
    .is_err());
    assert_eq!(memory.peek(&a), Some(b"original".as_slice()));
}

#[test]
fn transaction_quota_is_atomic_across_partitions_and_shared_stores() {
    let profile = Profile::new("quota");
    let ledger = Arc::new(Mutex::new(QuotaLedger::new(10)));
    let mut disk = DiskStore::open(profile.0.join("main"), 10000).unwrap();
    let mut other = DiskStore::open(profile.0.join("other"), 10000).unwrap();
    disk.attach_quota(ledger.clone());
    other.attach_quota(ledger.clone());
    let a = key("one", "a");
    let b = key("one", "b");
    let c = key("two", "c");
    other.put(a.clone(), vec![1; 3]).unwrap(); // five bytes
    disk.put(b.clone(), vec![2; 2]).unwrap(); // four bytes
    let original = fs::read(profile.0.join("main/log")).unwrap();
    assert_eq!(
        disk.transaction(|next| {
            next.put(c.clone(), vec![3; 3])?;
            next.put(b.clone(), vec![4; 4])
        }),
        Err(Error::QuotaExceeded)
    );
    assert_eq!(disk.get(&c), None);
    assert_eq!(ledger.lock().unwrap().usage(&c.bucket.storage), 0);
    assert_eq!(ledger.lock().unwrap().usage(&a.bucket.storage), 9);
    assert_eq!(fs::read(profile.0.join("main/log")).unwrap(), original);
    disk.transaction(|next| {
        next.delete(&b);
        next.put(a.clone(), vec![5; 3])?;
        next.put(c.clone(), vec![6; 3])
    })
    .unwrap();
    assert_eq!(ledger.lock().unwrap().usage(&a.bucket.storage), 10);
    assert_eq!(ledger.lock().unwrap().usage(&c.bucket.storage), 5);
    disk.transaction(|next| {
        next.delete_bucket(&a.bucket);
        next.clear_namespace(&c.bucket, "n");
        Ok(())
    })
    .unwrap();
    assert_eq!(ledger.lock().unwrap().usage(&a.bucket.storage), 5);
    assert_eq!(ledger.lock().unwrap().usage(&c.bucket.storage), 0);
    drop(disk);
    assert!(DiskStore::open(profile.0.join("main"), 10000)
        .unwrap()
        .memory()
        .entries_in("n")
        .is_empty());
}

#[test]
fn checkpoints_mixed_legacy_and_batch_records_reopen_without_resurrection() {
    let profile = Profile::new("checkpoint");
    let a = key("one", "a");
    let b = key("one", "b");
    let mut disk = profile.open();
    disk.put(a.clone(), b"legacy".to_vec()).unwrap();
    disk.transaction(|next| {
        next.delete(&a);
        next.put(b.clone(), b"batch".to_vec())
    })
    .unwrap();
    // Simulate a crash after snapshot rename but before log reset.
    let pre_checkpoint_log = fs::read(profile.0.join("log")).unwrap();
    disk.checkpoint().unwrap();
    drop(disk);
    fs::write(profile.0.join("log"), pre_checkpoint_log).unwrap();
    let mut disk = profile.open();
    assert_eq!(disk.get(&a), None);
    assert_eq!(disk.get(&b), Some(b"batch".to_vec()));
    for _ in 0..3 {
        disk.checkpoint().unwrap();
    }
    disk.transaction(|next| {
        next.delete(&b);
        next.put(a.clone(), b"after".to_vec())
    })
    .unwrap();
    disk.delete(&a).unwrap();
    disk.put(b.clone(), b"final".to_vec()).unwrap();
    drop(disk);
    let mut disk = profile.open();
    assert_eq!(disk.get(&a), None);
    assert_eq!(disk.get(&b), Some(b"final".to_vec()));
    assert_eq!(disk.memory().buckets(&b.bucket.storage), vec![b.bucket]);
}
