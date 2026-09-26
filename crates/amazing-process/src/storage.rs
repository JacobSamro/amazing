//! Browser-owned storage process. A renderer never receives this connection or
//! a filesystem path; the browser mints the partition in each request.
use crate::protocol::{
    StorageEntry, StoragePartition, StoragePut, StorageReceiver, StorageService,
};
use amazing_ipc::codec;
use amazing_ipc::codec::ByteBuf;
use amazing_ipc::{Connection, Error};
use amazing_storage::core::{BucketKey, DiskStore, EntryKey, StorageKey};
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

fn storage_error(error: amazing_storage::core::Error) -> Error {
    Error::Remote(format!("storage: {error:?}"))
}

struct Endpoint(Mutex<DiskStore>);
impl StorageService for Endpoint {
    fn put(&self, request: StoragePut) -> Result<bool, Error> {
        if request.value.0.len() > MAX_VALUE_BYTES {
            return Err(Error::Codec(codec::Error::Limit));
        }
        self.0
            .lock()
            .unwrap()
            .put(key(request.entry)?, request.value.0)
            .map_err(storage_error)?;
        Ok(true)
    }
    fn get(&self, request: StorageEntry) -> Result<Option<ByteBuf>, Error> {
        Ok(self.0.lock().unwrap().get(&key(request)?).map(ByteBuf))
    }
    fn delete(&self, request: StorageEntry) -> Result<bool, Error> {
        self.0
            .lock()
            .unwrap()
            .delete(&key(request)?)
            .map_err(storage_error)
    }
    fn checkpoint(&self, (): ()) -> Result<bool, Error> {
        self.0.lock().unwrap().checkpoint().map_err(storage_error)?;
        Ok(true)
    }
}

pub fn serve_storage(connection: &Arc<Connection>, root: &Path, quota: u64) -> Result<(), Error> {
    let disk = DiskStore::open(root, quota).map_err(storage_error)?;
    connection.bind(13, StorageReceiver(Endpoint(Mutex::new(disk))));
    connection.bind(0, crate::Ping);
    Ok(())
}
