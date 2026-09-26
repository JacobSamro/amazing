//! Browser-owned storage process. A renderer never receives this connection or
//! a filesystem path; the browser mints the partition in each request.
use crate::protocol::{
    LockClient, LockInfo as WireLockInfo, LockMode as WireLockMode, LockOptions as WireLockOptions,
    LockOutcome, LockQuery, LockRelease, LockRequest, LockSnapshot, StorageEntry,
    StoragePartition, StoragePut, StorageReceiver, StorageService,
};
use amazing_ipc::codec;
use amazing_ipc::codec::ByteBuf;
use amazing_ipc::{Connection, Error};
use amazing_storage::core::{BucketKey, DiskStore, EntryKey, StorageKey};
use amazing_storage::locks::{self, LockCoordinator};
use std::path::Path;
use std::sync::{Arc, Mutex};

const MAX_KEY_BYTES: usize = 4096;
const MAX_VALUE_BYTES: usize = codec::MAX_FRAME - 4096;

fn key(entry: StorageEntry) -> Result<EntryKey, Error> {
    let StorageEntry {
        partition,
        namespace,
        key,
    } = entry;
    let StoragePartition {
        origin,
        top_level_site,
        bucket,
    } = partition;
    if origin.is_empty()
        || top_level_site.is_empty()
        || bucket.is_empty()
        || origin.len() > MAX_KEY_BYTES
        || top_level_site.len() > MAX_KEY_BYTES
        || bucket.len() > MAX_KEY_BYTES
        || namespace.len() > MAX_KEY_BYTES
        || key.0.len() > MAX_KEY_BYTES
    {
        return Err(Error::Codec(codec::Error::Limit));
    }
    Ok(EntryKey {
        bucket: BucketKey::new(StorageKey::new(origin, top_level_site), bucket),
        namespace,
        key: key.0,
    })
}

fn lock_key(partition: StoragePartition) -> Result<StorageKey, Error> {
    if partition.origin.is_empty()
        || partition.top_level_site.is_empty()
        || partition.origin.len() > MAX_KEY_BYTES
        || partition.top_level_site.len() > MAX_KEY_BYTES
    {
        return Err(Error::Codec(codec::Error::Limit));
    }
    Ok(StorageKey::new(partition.origin, partition.top_level_site))
}

fn lock_mode(mode: WireLockMode) -> locks::Mode {
    match mode {
        WireLockMode::Shared(()) => locks::Mode::Shared,
        WireLockMode::Exclusive(()) => locks::Mode::Exclusive,
    }
}

fn wire_mode(mode: locks::Mode) -> WireLockMode {
    match mode {
        locks::Mode::Shared => WireLockMode::Shared(()),
        locks::Mode::Exclusive => WireLockMode::Exclusive(()),
    }
}

fn wire_lock(info: locks::LockInfo) -> WireLockInfo {
    WireLockInfo {
        id: info.id,
        client: info.client_id,
        name: info.name,
        mode: wire_mode(info.mode),
    }
}

fn storage_error(error: amazing_storage::core::Error) -> Error {
    Error::Remote(format!("storage: {error:?}"))
}

fn lock_error(error: locks::Error) -> Error {
    Error::Remote(format!("locks: {error:?}"))
}

struct Endpoint {
    disk: Mutex<DiskStore>,
    locks: Mutex<LockCoordinator>,
}

impl StorageService for Endpoint {
    fn put(&self, request: StoragePut) -> Result<bool, Error> {
        if request.value.0.len() > MAX_VALUE_BYTES {
            return Err(Error::Codec(codec::Error::Limit));
        }
        self.disk
            .lock()
            .unwrap()
            .put(key(request.entry)?, request.value.0)
            .map_err(storage_error)?;
        Ok(true)
    }
    fn get(&self, request: StorageEntry) -> Result<Option<ByteBuf>, Error> {
        Ok(self.disk.lock().unwrap().get(&key(request)?).map(ByteBuf))
    }
    fn delete(&self, request: StorageEntry) -> Result<bool, Error> {
        self.disk
            .lock()
            .unwrap()
            .delete(&key(request)?)
            .map_err(storage_error)
    }
    fn checkpoint(&self, (): ()) -> Result<bool, Error> {
        self.disk
            .lock()
            .unwrap()
            .checkpoint()
            .map_err(storage_error)?;
        Ok(true)
    }
    fn lock_request(&self, request: LockRequest) -> Result<LockOutcome, Error> {
        let key = lock_key(request.partition)?;
        if request.name.len() > MAX_KEY_BYTES {
            return Err(Error::Codec(codec::Error::Limit));
        }
        let outcome = self
            .locks
            .lock()
            .unwrap()
            .request(
                key,
                request.client,
                request.name,
                lock_mode(request.mode),
                locks::Options {
                    if_available: request.options.if_available,
                    steal: request.options.steal,
                },
            )
            .map_err(lock_error)?;
        Ok(LockOutcome {
            id: outcome.id,
            granted: outcome.granted,
            stolen: outcome.stolen,
        })
    }
    fn lock_release(&self, request: LockRelease) -> Result<Vec<u64>, Error> {
        Ok(self.locks.lock().unwrap().release(request.id))
    }
    fn lock_release_client(&self, request: LockClient) -> Result<Vec<u64>, Error> {
        Ok(self.locks.lock().unwrap().release_client(request.client))
    }
    fn lock_query(&self, request: LockQuery) -> Result<LockSnapshot, Error> {
        let key = lock_key(request.partition)?;
        let snapshot = self.locks.lock().unwrap().query(&key);
        Ok(LockSnapshot {
            held: snapshot.held.into_iter().map(wire_lock).collect(),
            pending: snapshot.pending.into_iter().map(wire_lock).collect(),
        })
    }
}

pub fn serve_storage(connection: &Arc<Connection>, root: &Path, quota: u64) -> Result<(), Error> {
    let disk = DiskStore::open(root, quota).map_err(storage_error)?;
    connection.bind(
        13,
        StorageReceiver(Endpoint {
            disk: Mutex::new(disk),
            locks: Mutex::new(LockCoordinator::default()),
        }),
    );
    connection.bind(0, crate::Ping);
    Ok(())
}
