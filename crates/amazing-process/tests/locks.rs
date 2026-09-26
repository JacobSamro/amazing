//! Web Locks arbitration over the storage process seam (IPC-11c).
use amazing_ipc::Connection;
use amazing_process::protocol::{
    LockClient, LockMode, LockOptions, LockQuery, LockRelease, LockRequest, StoragePartition,
    StorageProxy,
};
use amazing_process::storage::serve_storage;
use std::time::Duration;

fn partition() -> StoragePartition {
    StoragePartition {
        origin: "https://a.test".into(),
        top_level_site: "https://a.test".into(),
        bucket: "default".into(),
    }
}

fn request(
    client: u64,
    mode: LockMode,
    if_available: bool,
    steal: bool,
) -> LockRequest {
    LockRequest {
        partition: partition(),
        client,
        name: "db".into(),
        mode,
        options: LockOptions {
            if_available,
            steal,
        },
    }
}

#[test]
fn storage_process_arbitrates_web_locks_fifo_and_release() {
    let (a, b) = amazing_ipc::memory_pair();
    let server = Connection::new(a);
    let client = Connection::new(b);
    let root = std::env::temp_dir().join(format!("amazing-locks-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    serve_storage(&server, &root, 10 * 1024 * 1024).unwrap();
    let proxy = StorageProxy::new(client, Duration::from_secs(2));

    let first = proxy
        .lock_request(request(1, LockMode::Exclusive(()), false, false))
        .unwrap();
    assert_eq!(first.granted, vec![first.id.unwrap()]);
    let second = proxy
        .lock_request(request(2, LockMode::Shared(()), false, false))
        .unwrap();
    assert!(second.granted.is_empty());

    let snapshot = proxy.lock_query(LockQuery { partition: partition() }).unwrap();
    assert_eq!(snapshot.held.len(), 1);
    assert_eq!(snapshot.pending.len(), 1);

    let granted = proxy
        .lock_release(LockRelease {
            client: 1,
            id: first.id.unwrap(),
        })
        .unwrap();
    assert_eq!(granted, vec![second.id.unwrap()]);
    let snapshot = proxy.lock_query(LockQuery { partition: partition() }).unwrap();
    assert_eq!(snapshot.held.len(), 1);
    assert!(snapshot.pending.is_empty());

    // Disconnect release drains the remaining holder.
    assert!(proxy
        .lock_release_client(LockClient { client: 2 })
        .unwrap()
        .is_empty());
    assert!(proxy
        .lock_query(LockQuery { partition: partition() })
        .unwrap()
        .held
        .is_empty());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn if_available_and_steal_match_the_coordinator() {
    let (a, b) = amazing_ipc::memory_pair();
    let server = Connection::new(a);
    let client = Connection::new(b);
    let root = std::env::temp_dir().join(format!("amazing-locks2-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    serve_storage(&server, &root, 10 * 1024 * 1024).unwrap();
    let proxy = StorageProxy::new(client, Duration::from_secs(2));

    let held = proxy
        .lock_request(request(1, LockMode::Exclusive(()), false, false))
        .unwrap();
    let unavailable = proxy
        .lock_request(request(2, LockMode::Shared(()), true, false))
        .unwrap();
    assert_eq!(unavailable.id, None);
    assert!(unavailable.granted.is_empty());

    let stolen = proxy
        .lock_request(request(3, LockMode::Exclusive(()), false, true))
        .unwrap();
    assert_eq!(stolen.stolen, vec![held.id.unwrap()]);
    assert_eq!(stolen.granted, vec![stolen.id.unwrap()]);

    // Invalid names are rejected before arbitration.
    let mut bad = request(4, LockMode::Shared(()), false, false);
    bad.name = "-reserved".into();
    assert!(proxy.lock_request(bad).is_err());

    let _ = std::fs::remove_dir_all(&root);
}
