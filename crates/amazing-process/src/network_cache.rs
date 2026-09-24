//! Bounded, versioned disk snapshot for the network process's HTTP cache.
use amazing_fetch::cache::{
    CacheEntry, CacheKind, CacheLimits, CacheStorage, CacheStore, MemoryCacheStorage,
};
use amazing_fetch::service::CachePersistence;
use amazing_fetch::HeaderList;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 8] = b"AMHC0001";
const MAX_FILE: u64 = 32 * 1024 * 1024;
const MAX_FIELD: usize = 8 * 1024 * 1024;

pub struct DiskCache {
    path: PathBuf,
}

impl DiskCache {
    /// A corrupt or obsolete snapshot is discarded. The policy store checks
    /// all restored entry sizes before any entry becomes available to Fetch.
    pub fn open(dir: impl AsRef<Path>, limits: CacheLimits) -> io::Result<(CacheStore, Self)> {
        let dir = dir.as_ref();
        fs::create_dir_all(dir)?;
        let disk = Self {
            path: dir.join("http-cache-v1.bin"),
        };
        let storage = match File::open(&disk.path) {
            Ok(file) => match read_snapshot(file) {
                Ok(storage) => storage,
                Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                    MemoryCacheStorage::default()
                }
                Err(error) => return Err(error),
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => MemoryCacheStorage::default(),
            Err(error) => return Err(error),
        };
        let store = CacheStore::with_storage(storage, CacheKind::Private, limits)
            .or_else(|_| CacheStore::new(CacheKind::Private, limits))
            .map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidInput, "invalid HTTP cache limits")
            })?;
        Ok((store, disk))
    }

    /// Write a complete snapshot to a sibling file before replacing the old
    /// one. Failed writes leave the previous cache available on restart.
    pub fn save(&self, store: &CacheStore) -> io::Result<()> {
        let bytes = encode(store)?;
        let temp = self.path.with_extension("tmp");
        let result = (|| {
            let mut file = File::create(&temp)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temp, &self.path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result
    }
}

impl CachePersistence for DiskCache {
    fn save(&self, cache: &CacheStore) -> io::Result<()> {
        DiskCache::save(self, cache)
    }
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid HTTP cache snapshot")
}
fn put_u32(out: &mut Vec<u8>, value: usize) -> io::Result<()> {
    out.extend_from_slice(&u32::try_from(value).map_err(|_| invalid())?.to_le_bytes());
    Ok(())
}
fn put_bytes(out: &mut Vec<u8>, value: &[u8]) -> io::Result<()> {
    if value.len() > MAX_FIELD {
        return Err(invalid());
    }
    put_u32(out, value.len())?;
    out.extend_from_slice(value);
    if out.len() as u64 > MAX_FILE {
        return Err(invalid());
    }
    Ok(())
}
fn put_string(out: &mut Vec<u8>, value: &str) -> io::Result<()> {
    put_bytes(out, value.as_bytes())
}

fn encode(store: &CacheStore) -> io::Result<Vec<u8>> {
    let mut out = MAGIC.to_vec();
    put_u32(&mut out, store.len())?;
    for index in 0..store.len() {
        let entry = store.storage().get(index).ok_or_else(invalid)?;
        put_string(&mut out, &entry.uri)?;
        put_string(&mut out, &entry.legacy_key)?;
        put_u32(&mut out, entry.vary.len())?;
        for (name, value) in &entry.vary {
            put_string(&mut out, name)?;
            out.push(u8::from(value.is_some()));
            if let Some(value) = value {
                put_string(&mut out, value)?;
            }
        }
        out.extend_from_slice(&entry.response.status.to_le_bytes());
        let headers: Vec<_> = entry.response.headers.iter().collect();
        put_u32(&mut out, headers.len())?;
        for (name, value) in headers {
            put_string(&mut out, name)?;
            put_string(&mut out, value)?;
        }
        put_bytes(&mut out, &entry.response.body)?;
        out.extend_from_slice(&entry.request_time.to_le_bytes());
        out.extend_from_slice(&entry.response_time.to_le_bytes());
        if out.len() as u64 > MAX_FILE {
            return Err(invalid());
        }
    }
    Ok(out)
}

struct Decoder<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl<'a> Decoder<'a> {
    fn take(&mut self, len: usize) -> io::Result<&'a [u8]> {
        let end = self.offset.checked_add(len).ok_or_else(invalid)?;
        let value = self.bytes.get(self.offset..end).ok_or_else(invalid)?;
        self.offset = end;
        Ok(value)
    }
    fn u32(&mut self) -> io::Result<usize> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().map_err(|_| invalid())?) as usize)
    }
    fn bytes(&mut self) -> io::Result<&'a [u8]> {
        let len = self.u32()?;
        if len > MAX_FIELD {
            return Err(invalid());
        }
        self.take(len)
    }
    fn string(&mut self) -> io::Result<String> {
        String::from_utf8(self.bytes()?.to_vec()).map_err(|_| invalid())
    }
    fn i64(&mut self) -> io::Result<i64> {
        Ok(i64::from_le_bytes(
            self.take(8)?.try_into().map_err(|_| invalid())?,
        ))
    }
}

fn read_snapshot(file: File) -> io::Result<MemoryCacheStorage> {
    if file.metadata()?.len() > MAX_FILE {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_FILE {
        return Err(invalid());
    }
    let mut decoder = Decoder {
        bytes: &bytes,
        offset: 0,
    };
    if decoder.take(MAGIC.len())? != MAGIC {
        return Err(invalid());
    }
    let count = decoder.u32()?;
    if count > 64 {
        return Err(invalid());
    }
    let mut storage = MemoryCacheStorage::default();
    for _ in 0..count {
        let uri = decoder.string()?;
        let legacy_key = decoder.string()?;
        let vary_count = decoder.u32()?;
        if vary_count > 32 {
            return Err(invalid());
        }
        let mut vary = Vec::with_capacity(vary_count);
        for _ in 0..vary_count {
            let name = decoder.string()?;
            let value = match decoder.take(1)?[0] {
                0 => None,
                1 => Some(decoder.string()?),
                _ => return Err(invalid()),
            };
            vary.push((name, value));
        }
        let status = u16::from_le_bytes(decoder.take(2)?.try_into().map_err(|_| invalid())?);
        let header_count = decoder.u32()?;
        if header_count > 4096 {
            return Err(invalid());
        }
        let mut headers = HeaderList::default();
        for _ in 0..header_count {
            headers
                .append(&decoder.string()?, &decoder.string()?)
                .map_err(|_| invalid())?;
        }
        let body = decoder.bytes()?.to_vec();
        let request_time = decoder.i64()?;
        let response_time = decoder.i64()?;
        if request_time > response_time {
            return Err(invalid());
        }
        storage.push(CacheEntry {
            uri,
            legacy_key,
            vary,
            response: amazing_fetch::cache::CacheResponse {
                status,
                headers,
                body,
            },
            request_time,
            response_time,
        });
    }
    if decoder.offset != bytes.len() {
        return Err(invalid());
    }
    Ok(storage)
}
