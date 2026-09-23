//! Pure IndexedDB engine. Script promises and event dispatch remain in bindings.
use crate::core::{BucketKey, DiskStore, EntryKey, Error as StorageError, StorageKey};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
mod codec;

#[derive(Clone, Debug, PartialEq)]
pub enum StructuredValue {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Date(f64),
    Binary(Vec<u8>),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}
#[derive(Clone, Debug)]
pub enum Key {
    Number(f64),
    Date(f64),
    String(String),
    Binary(Vec<u8>),
    Array(Vec<Key>),
}
impl Key {
    pub fn number(n: f64) -> Result<Self> {
        if n.is_nan() {
            Err(Error::Data)
        } else {
            Ok(Self::Number(if n == 0.0 { 0.0 } else { n }))
        }
    }
    pub fn date(n: f64) -> Result<Self> {
        if n.is_nan() {
            Err(Error::Data)
        } else {
            Ok(Self::Date(n))
        }
    }
    pub fn from_value(value: &StructuredValue) -> Result<Self> {
        match value {
            StructuredValue::Number(n) => Self::number(*n),
            StructuredValue::Date(n) => Self::date(*n),
            StructuredValue::String(s) => Ok(Self::String(s.clone())),
            StructuredValue::Binary(b) => Ok(Self::Binary(b.clone())),
            StructuredValue::Array(a) => Ok(Self::Array(
                a.iter().map(Self::from_value).collect::<Result<_>>()?,
            )),
            _ => Err(Error::Data),
        }
    }
    fn kind(&self) -> u8 {
        match self {
            Self::Number(_) => 0,
            Self::Date(_) => 1,
            Self::String(_) => 2,
            Self::Binary(_) => 3,
            Self::Array(_) => 4,
        }
    }
}
impl PartialEq for Key {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Key {}
impl PartialOrd for Key {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Key {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.kind().cmp(&other.kind()) {
            Ordering::Equal => {}
            other => return other,
        }
        match (self, other) {
            (Self::Number(a), Self::Number(b)) | (Self::Date(a), Self::Date(b)) => a.total_cmp(b),
            (Self::String(a), Self::String(b)) => a.encode_utf16().cmp(b.encode_utf16()),
            (Self::Binary(a), Self::Binary(b)) => a.cmp(b),
            (Self::Array(a), Self::Array(b)) => a.cmp(b),
            _ => unreachable!(),
        }
    }
}
#[derive(Clone, Debug)]
pub struct KeyRange {
    pub lower: Option<Key>,
    pub upper: Option<Key>,
    pub lower_open: bool,
    pub upper_open: bool,
}
impl KeyRange {
    pub fn only(key: Key) -> Self {
        Self {
            lower: Some(key.clone()),
            upper: Some(key),
            lower_open: false,
            upper_open: false,
        }
    }
    pub fn bound(lower: Key, upper: Key, lower_open: bool, upper_open: bool) -> Result<Self> {
        if lower > upper || (lower == upper && (lower_open || upper_open)) {
            return Err(Error::Data);
        }
        Ok(Self {
            lower: Some(lower),
            upper: Some(upper),
            lower_open,
            upper_open,
        })
    }
    pub fn includes(&self, key: &Key) -> bool {
        self.lower
            .as_ref()
            .is_none_or(|v| if self.lower_open { key > v } else { key >= v })
            && self
                .upper
                .as_ref()
                .is_none_or(|v| if self.upper_open { key < v } else { key <= v })
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Error {
    Data,
    Constraint,
    NotFound,
    InvalidState,
    ReadOnly,
    TransactionInactive,
    Abort,
    Version,
    Blocked,
    QuotaExceeded,
}
impl From<StorageError> for Error {
    fn from(_: StorageError) -> Self {
        Self::QuotaExceeded
    }
}
pub type Result<T> = std::result::Result<T, Error>;
#[derive(Clone, Debug)]
pub enum KeyPath {
    Single(String),
    Compound(Vec<String>),
}
fn property<'a>(value: &'a StructuredValue, path: &str) -> Option<&'a StructuredValue> {
    if path.is_empty() {
        return Some(value);
    }
    let mut current = value;
    for component in path.split('.') {
        current = match current {
            StructuredValue::Object(map) => map.get(component)?,
            _ => return None,
        };
    }
    Some(current)
}
fn assign(value: &mut StructuredValue, path: &str, key: &Key) -> Result<()> {
    let mut parts = path.split('.').peekable();
    let mut current = value;
    while let Some(part) = parts.next() {
        let StructuredValue::Object(map) = current else {
            return Err(Error::Data);
        };
        if parts.peek().is_none() {
            map.insert(part.to_owned(), key_value(key));
            return Ok(());
        }
        current = map
            .entry(part.to_owned())
            .or_insert_with(|| StructuredValue::Object(BTreeMap::new()));
    }
    Err(Error::Data)
}
fn key_value(key: &Key) -> StructuredValue {
    match key {
        Key::Number(n) => StructuredValue::Number(*n),
        Key::Date(n) => StructuredValue::Date(*n),
        Key::String(s) => StructuredValue::String(s.clone()),
        Key::Binary(b) => StructuredValue::Binary(b.clone()),
        Key::Array(a) => StructuredValue::Array(a.iter().map(key_value).collect()),
    }
}
fn extract(value: &StructuredValue, path: &KeyPath) -> Result<Option<Key>> {
    match path {
        KeyPath::Single(p) => property(value, p).map(Key::from_value).transpose(),
        KeyPath::Compound(paths) => {
            let mut keys = Vec::new();
            for p in paths {
                let Some(v) = property(value, p) else {
                    return Ok(None);
                };
                keys.push(Key::from_value(v)?);
            }
            Ok(Some(Key::Array(keys)))
        }
    }
}
#[derive(Clone, Debug)]
pub struct Index {
    pub key_path: KeyPath,
    pub unique: bool,
    pub multi_entry: bool,
    entries: BTreeMap<Key, BTreeSet<Key>>,
}
impl Index {
    fn keys(&self, value: &StructuredValue) -> Result<Vec<Key>> {
        if self.multi_entry {
            if let KeyPath::Single(path) = &self.key_path {
                let Some(source) = property(value, path) else {
                    return Ok(Vec::new());
                };
                if let StructuredValue::Array(values) = source {
                    return Ok(values
                        .iter()
                        .filter_map(|value| Key::from_value(value).ok())
                        .collect::<BTreeSet<_>>()
                        .into_iter()
                        .collect());
                }
            }
        }
        let Some(key) = extract(value, &self.key_path)? else {
            return Ok(Vec::new());
        };
        if self.multi_entry {
            if let Key::Array(keys) = key {
                return Ok(keys
                    .into_iter()
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect());
            }
        }
        Ok(vec![key])
    }
    fn add(&mut self, primary: &Key, value: &StructuredValue) -> Result<()> {
        for key in self.keys(value)? {
            let entries = self.entries.entry(key).or_default();
            if self.unique && !entries.is_empty() && !entries.contains(primary) {
                return Err(Error::Constraint);
            }
            entries.insert(primary.clone());
        }
        Ok(())
    }
    fn remove(&mut self, primary: &Key, value: &StructuredValue) {
        if let Ok(keys) = self.keys(value) {
            for key in keys {
                if let Some(entries) = self.entries.get_mut(&key) {
                    entries.remove(primary);
                    if entries.is_empty() {
                        self.entries.remove(&key);
                    }
                }
            }
        }
    }
}
#[derive(Clone, Debug)]
pub struct ObjectStore {
    pub key_path: Option<KeyPath>,
    pub auto_increment: bool,
    generator: u64,
    records: BTreeMap<Key, StructuredValue>,
    indexes: BTreeMap<String, Index>,
}
impl ObjectStore {
    pub fn new(key_path: Option<KeyPath>, auto_increment: bool) -> Result<Self> {
        if auto_increment
            && (matches!(key_path, Some(KeyPath::Compound(_)))
                || matches!(key_path,Some(KeyPath::Single(ref p)) if p.is_empty()))
        {
            return Err(Error::InvalidState);
        }
        Ok(Self {
            key_path,
            auto_increment,
            generator: 1,
            records: BTreeMap::new(),
            indexes: BTreeMap::new(),
        })
    }
    pub fn create_index(
        &mut self,
        name: impl Into<String>,
        key_path: KeyPath,
        unique: bool,
        multi_entry: bool,
    ) -> Result<()> {
        let name = name.into();
        if self.indexes.contains_key(&name)
            || (multi_entry && matches!(key_path, KeyPath::Compound(_)))
        {
            return Err(Error::Constraint);
        }
        let mut index = Index {
            key_path,
            unique,
            multi_entry,
            entries: BTreeMap::new(),
        };
        for (key, value) in &self.records {
            index.add(key, value)?;
        }
        self.indexes.insert(name, index);
        Ok(())
    }
    pub fn delete_index(&mut self, name: &str) -> Result<()> {
        self.indexes.remove(name).map(|_| ()).ok_or(Error::NotFound)
    }
    pub fn get(&self, key: &Key) -> Option<&StructuredValue> {
        self.records.get(key)
    }
    pub fn count(&self, range: Option<&KeyRange>) -> usize {
        self.records
            .keys()
            .filter(|key| range.is_none_or(|r| r.includes(key)))
            .count()
    }
    pub fn put(
        &mut self,
        mut value: StructuredValue,
        explicit: Option<Key>,
        overwrite: bool,
    ) -> Result<Key> {
        if self.key_path.is_some() && explicit.is_some() {
            return Err(Error::Data);
        }
        let extracted = self
            .key_path
            .as_ref()
            .map(|p| extract(&value, p))
            .transpose()?
            .flatten();
        let key = match extracted.or(explicit) {
            Some(k) => k,
            None if self.auto_increment => {
                if self.generator > 9_007_199_254_740_992 {
                    return Err(Error::Constraint);
                }
                let key = Key::Number(self.generator as f64);
                if let Some(KeyPath::Single(path)) = &self.key_path {
                    assign(&mut value, path, &key)?;
                }
                key
            }
            None => return Err(Error::Data),
        };
        if !overwrite && self.records.contains_key(&key) {
            return Err(Error::Constraint);
        }
        // Index mutation is atomic even on a uniqueness failure.
        let mut next = self.clone();
        if let Some(old) = next.records.get(&key) {
            for index in next.indexes.values_mut() {
                index.remove(&key, old);
            }
        }
        for index in next.indexes.values_mut() {
            index.add(&key, &value)?;
        }
        next.records.insert(key.clone(), value);
        if let Key::Number(n) = key {
            if n >= next.generator as f64 && n < 9_007_199_254_740_992.0 {
                next.generator = n.floor() as u64 + 1;
            }
        }
        *self = next;
        Ok(key)
    }
    pub fn delete(&mut self, key: &Key) -> bool {
        if let Some(old) = self.records.remove(key) {
            for index in self.indexes.values_mut() {
                index.remove(key, &old);
            }
            true
        } else {
            false
        }
    }
    pub fn clear(&mut self) {
        self.records.clear();
        for index in self.indexes.values_mut() {
            index.entries.clear();
        }
    }
    pub fn index(&self, name: &str) -> Result<&Index> {
        self.indexes.get(name).ok_or(Error::NotFound)
    }
    pub fn cursor(&self, range: Option<&KeyRange>, direction: Direction) -> Cursor {
        let entries = self
            .records
            .iter()
            .filter(|(k, _)| range.is_none_or(|r| r.includes(k)))
            .map(|(k, v)| (k.clone(), k.clone(), v.clone()))
            .collect();
        Cursor::new(entries, direction, false)
    }
}
impl Index {
    pub fn get<'a>(&self, store: &'a ObjectStore, key: &Key) -> Option<&'a StructuredValue> {
        self.entries
            .get(key)?
            .iter()
            .next()
            .and_then(|pk| store.records.get(pk))
    }
    pub fn count(&self, range: Option<&KeyRange>) -> usize {
        self.entries
            .iter()
            .filter(|(k, _)| range.is_none_or(|r| r.includes(k)))
            .map(|(_, v)| v.len())
            .sum()
    }
    pub fn cursor(
        &self,
        store: &ObjectStore,
        range: Option<&KeyRange>,
        direction: Direction,
    ) -> Cursor {
        let entries = self
            .entries
            .iter()
            .filter(|(k, _)| range.is_none_or(|r| r.includes(k)))
            .flat_map(|(k, pks)| {
                pks.iter().filter_map(|pk| {
                    store
                        .records
                        .get(pk)
                        .map(|v| (k.clone(), pk.clone(), v.clone()))
                })
            })
            .collect();
        Cursor::new(entries, direction, true)
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Direction {
    Next,
    NextUnique,
    Prev,
    PrevUnique,
}
#[derive(Clone, Debug)]
pub struct CursorRecord {
    pub key: Key,
    pub primary_key: Key,
    pub value: StructuredValue,
}
#[derive(Clone, Debug)]
pub struct Cursor {
    records: Vec<CursorRecord>,
    position: Option<usize>,
    direction: Direction,
    from_index: bool,
}
impl Cursor {
    fn new(
        mut entries: Vec<(Key, Key, StructuredValue)>,
        direction: Direction,
        from_index: bool,
    ) -> Self {
        entries.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
        if matches!(direction, Direction::Prev | Direction::PrevUnique) {
            entries.reverse();
        }
        if matches!(direction, Direction::NextUnique | Direction::PrevUnique) {
            let mut seen = BTreeSet::new();
            entries.retain(|e| seen.insert(e.0.clone()));
        }
        Self {
            records: entries
                .into_iter()
                .map(|(key, primary_key, value)| CursorRecord {
                    key,
                    primary_key,
                    value,
                })
                .collect(),
            position: None,
            direction,
            from_index,
        }
    }
    pub fn current(&self) -> Option<&CursorRecord> {
        self.position.and_then(|i| self.records.get(i))
    }
    pub fn advance(&mut self, count: usize) -> Result<Option<&CursorRecord>> {
        if count == 0 {
            return Err(Error::Data);
        }
        let next = self.position.map_or(count - 1, |i| i.saturating_add(count));
        self.position = Some(next);
        Ok(self.current())
    }
    pub fn continue_to(&mut self, key: Option<&Key>) -> Result<Option<&CursorRecord>> {
        if key.is_none() {
            return self.advance(1);
        }
        let key = key.unwrap();
        if let Some(current) = self.current() {
            if match self.direction {
                Direction::Next | Direction::NextUnique => key <= &current.key,
                _ => key >= &current.key,
            } {
                return Err(Error::Data);
            }
        }
        let start = self.position.map_or(0, |i| i + 1);
        self.position = Some(
            (start..self.records.len())
                .find(|&i| match self.direction {
                    Direction::Next | Direction::NextUnique => &self.records[i].key >= key,
                    _ => &self.records[i].key <= key,
                })
                .unwrap_or(self.records.len()),
        );
        Ok(self.current())
    }
    pub fn continue_primary_key(
        &mut self,
        key: &Key,
        primary: &Key,
    ) -> Result<Option<&CursorRecord>> {
        if !self.from_index {
            return Err(Error::InvalidState);
        }
        if matches!(
            self.direction,
            Direction::NextUnique | Direction::PrevUnique
        ) {
            return Err(Error::InvalidState);
        }
        if let Some(current) = self.current() {
            let ordering = (&current.key, &current.primary_key).cmp(&(key, primary));
            if match self.direction {
                Direction::Next => ordering != Ordering::Less,
                Direction::Prev => ordering != Ordering::Greater,
                _ => false,
            } {
                return Err(Error::Data);
            }
        }
        let start = self.position.map_or(0, |i| i + 1);
        self.position = Some(
            (start..self.records.len())
                .find(|&i| {
                    let pair = (&self.records[i].key, &self.records[i].primary_key);
                    match self.direction {
                        Direction::Next => pair >= (key, primary),
                        Direction::Prev => pair <= (key, primary),
                        _ => false,
                    }
                })
                .unwrap_or(self.records.len()),
        );
        Ok(self.current())
    }
}
#[derive(Clone, Debug)]
pub struct Database {
    pub version: u64,
    stores: BTreeMap<String, ObjectStore>,
}
impl Database {
    pub fn new(version: u64) -> Self {
        Self {
            version,
            stores: BTreeMap::new(),
        }
    }
    pub fn create_store(
        &mut self,
        name: impl Into<String>,
        key_path: Option<KeyPath>,
        auto_increment: bool,
    ) -> Result<()> {
        let name = name.into();
        if self.stores.contains_key(&name) {
            return Err(Error::Constraint);
        }
        self.stores
            .insert(name, ObjectStore::new(key_path, auto_increment)?);
        Ok(())
    }
    pub fn delete_store(&mut self, name: &str) -> Result<()> {
        self.stores.remove(name).map(|_| ()).ok_or(Error::NotFound)
    }
    pub fn store(&self, name: &str) -> Result<&ObjectStore> {
        self.stores.get(name).ok_or(Error::NotFound)
    }
    pub fn store_mut(&mut self, name: &str) -> Result<&mut ObjectStore> {
        self.stores.get_mut(name).ok_or(Error::NotFound)
    }
    pub fn store_names(&self) -> Vec<&str> {
        self.stores.keys().map(String::as_str).collect()
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    ReadOnly,
    ReadWrite,
    VersionChange,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Durability {
    Strict,
    Relaxed,
    Default,
}
#[derive(Clone, Debug)]
pub struct Transaction {
    pub mode: Mode,
    pub durability: Durability,
    pub scope: BTreeSet<String>,
    working: Database,
    active: bool,
    aborted: bool,
}
impl Transaction {
    pub fn store(&self, name: &str) -> Result<&ObjectStore> {
        if !self.scope.contains(name) && self.mode != Mode::VersionChange {
            return Err(Error::NotFound);
        }
        self.working.store(name)
    }
    pub fn store_mut(&mut self, name: &str) -> Result<&mut ObjectStore> {
        if !self.active {
            return Err(Error::TransactionInactive);
        }
        if self.mode == Mode::ReadOnly {
            return Err(Error::ReadOnly);
        }
        if !self.scope.contains(name) && self.mode != Mode::VersionChange {
            return Err(Error::NotFound);
        }
        self.working.store_mut(name)
    }
    pub fn database_mut(&mut self) -> Result<&mut Database> {
        if self.mode != Mode::VersionChange {
            return Err(Error::InvalidState);
        }
        if !self.active {
            return Err(Error::TransactionInactive);
        }
        Ok(&mut self.working)
    }
    pub fn abort(&mut self) {
        self.active = false;
        self.aborted = true;
    }
    pub fn is_active(&self) -> bool {
        self.active
    }
}
#[derive(Clone, Debug)]
pub struct Connection {
    pub id: u64,
    pub key: StorageKey,
    pub name: String,
    pub version: u64,
    pub close_pending: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OpenEvent {
    VersionChange {
        connection: u64,
        old: u64,
        new: Option<u64>,
    },
    Blocked {
        old: u64,
        new: Option<u64>,
    },
    UpgradeNeeded {
        old: u64,
        new: u64,
    },
    Success {
        connection: u64,
        version: u64,
    },
}
#[derive(Clone, Debug)]
pub struct OpenResult {
    pub events: Vec<OpenEvent>,
    pub connection: Option<u64>,
    pub blocked: bool,
}
#[derive(Default)]
pub struct IndexedDb {
    databases: BTreeMap<(StorageKey, String), Database>,
    connections: BTreeMap<u64, Connection>,
    next_id: u64,
    active: Vec<(u64, u64, Mode, BTreeSet<String>)>,
    pending: VecDeque<(StorageKey, String, u64)>,
    pending_upgrade: BTreeMap<u64, u64>,
    events: VecDeque<OpenEvent>,
    disk: Option<DiskStore>,
}
impl IndexedDb {
    pub fn with_disk(disk: DiskStore) -> Result<Self> {
        let mut engine = Self::default();
        for (key, data) in disk.memory().entries_in("idb") {
            let name = String::from_utf8(key.key).map_err(|_| Error::Data)?;
            engine
                .databases
                .insert((key.bucket.storage, name), codec::decode(&data)?);
        }
        engine.disk = Some(disk);
        Ok(engine)
    }
    fn persist(&mut self, key: &StorageKey, name: &str, db: &Database) -> Result<()> {
        if let Some(disk) = &mut self.disk {
            let entry = EntryKey {
                bucket: BucketKey::new(key.clone(), "default"),
                namespace: "idb".into(),
                key: name.as_bytes().to_vec(),
            };
            disk.put(entry, codec::encode(db)).map_err(Error::from)?;
        }
        Ok(())
    }
    pub fn database(&self, key: &StorageKey, name: &str) -> Option<&Database> {
        self.databases.get(&(key.clone(), name.to_owned()))
    }
    pub fn database_mut(&mut self, key: &StorageKey, name: &str) -> Option<&mut Database> {
        self.databases.get_mut(&(key.clone(), name.to_owned()))
    }
    pub fn open(
        &mut self,
        key: StorageKey,
        name: impl Into<String>,
        version: Option<u64>,
    ) -> Result<OpenResult> {
        let name = name.into();
        let dbkey = (key.clone(), name.clone());
        let old = self.databases.get(&dbkey).map_or(0, |d| d.version);
        let new = version.unwrap_or(old.max(1));
        if new == 0 || new < old {
            return Err(Error::Version);
        }
        let mut events = Vec::new();
        if new > old {
            let blockers: Vec<_> = self
                .connections
                .values()
                .filter(|c| c.key == key && c.name == name && !c.close_pending)
                .map(|c| c.id)
                .collect();
            for id in &blockers {
                events.push(OpenEvent::VersionChange {
                    connection: *id,
                    old,
                    new: Some(new),
                });
            }
            if !blockers.is_empty() {
                events.push(OpenEvent::Blocked {
                    old,
                    new: Some(new),
                });
                self.pending.push_back((key, name, new));
                return Ok(OpenResult {
                    events,
                    connection: None,
                    blocked: true,
                });
            }
            self.databases
                .entry(dbkey)
                .or_insert_with(|| Database::new(0));
            events.push(OpenEvent::UpgradeNeeded { old, new });
        }
        self.next_id += 1;
        let id = self.next_id;
        self.connections.insert(
            id,
            Connection {
                id,
                key,
                name,
                version: new,
                close_pending: false,
            },
        );
        if new > old {
            self.pending_upgrade.insert(id, new);
        } else {
            events.push(OpenEvent::Success {
                connection: id,
                version: new,
            });
        }
        Ok(OpenResult {
            events,
            connection: Some(id),
            blocked: false,
        })
    }
    pub fn close(&mut self, id: u64) -> Result<()> {
        let connection = self.connections.remove(&id).ok_or(Error::NotFound)?;
        if self.pending_upgrade.remove(&id).is_some() {
            let key = (connection.key, connection.name);
            if self.databases.get(&key).is_some_and(|db| db.version == 0) {
                self.databases.remove(&key);
            }
        }
        Ok(())
    }
    pub fn connection(&self, id: u64) -> Option<&Connection> {
        self.connections.get(&id)
    }
    pub fn take_events(&mut self) -> Vec<OpenEvent> {
        self.events.drain(..).collect()
    }
    pub fn resume_pending(&mut self) -> Vec<OpenResult> {
        let mut ready = Vec::new();
        let pending = std::mem::take(&mut self.pending);
        for (key, name, version) in pending {
            let dbkey = (key.clone(), name.clone());
            if self
                .connections
                .values()
                .any(|c| c.key == key && c.name == name)
            {
                self.pending.push_back((key, name, version));
                continue;
            }
            let old = self.databases.get(&dbkey).map_or(0, |d| d.version);
            self.databases
                .entry(dbkey)
                .or_insert_with(|| Database::new(0));
            self.next_id += 1;
            let id = self.next_id;
            self.connections.insert(
                id,
                Connection {
                    id,
                    key,
                    name,
                    version,
                    close_pending: false,
                },
            );
            self.pending_upgrade.insert(id, version);
            ready.push(OpenResult {
                events: vec![OpenEvent::UpgradeNeeded { old, new: version }],
                connection: Some(id),
                blocked: false,
            });
        }
        ready
    }
    pub fn begin(
        &mut self,
        id: u64,
        scope: impl IntoIterator<Item = String>,
        mode: Mode,
        durability: Durability,
    ) -> Result<(u64, Transaction)> {
        let connection = self.connections.get(&id).ok_or(Error::NotFound)?;
        let scope: BTreeSet<_> = scope.into_iter().collect();
        let db = self
            .databases
            .get(&(connection.key.clone(), connection.name.clone()))
            .ok_or(Error::NotFound)?;
        if mode == Mode::VersionChange && !self.pending_upgrade.contains_key(&id) {
            return Err(Error::InvalidState);
        }
        if mode != Mode::VersionChange && self.pending_upgrade.contains_key(&id) {
            return Err(Error::Blocked);
        }
        if mode != Mode::VersionChange && scope.iter().any(|n| !db.stores.contains_key(n)) {
            return Err(Error::NotFound);
        }
        if mode == Mode::VersionChange
            && self
                .connections
                .values()
                .any(|c| c.key == connection.key && c.name == connection.name && c.id != id)
        {
            return Err(Error::Blocked);
        }
        if self.active.iter().any(|(_, cid, m, s)| {
            self.connections
                .get(cid)
                .is_some_and(|c| c.key == connection.key && c.name == connection.name)
                && (mode == Mode::VersionChange
                    || *m == Mode::VersionChange
                    || (mode == Mode::ReadWrite || *m == Mode::ReadWrite) && !s.is_disjoint(&scope))
        }) {
            return Err(Error::Blocked);
        }
        self.next_id += 1;
        let txid = self.next_id;
        self.active.push((txid, id, mode, scope.clone()));
        let mut working = db.clone();
        if mode == Mode::VersionChange {
            working.version = self.pending_upgrade[&id];
        }
        Ok((
            txid,
            Transaction {
                mode,
                durability,
                scope,
                working,
                active: true,
                aborted: false,
            },
        ))
    }
    pub fn finish(&mut self, id: u64, txid: u64, mut tx: Transaction, commit: bool) -> Result<()> {
        let connection = self.connections.get(&id).ok_or(Error::NotFound)?.clone();
        let pos = self
            .active
            .iter()
            .position(|(tid, cid, _, _)| *tid == txid && *cid == id)
            .ok_or(Error::TransactionInactive)?;
        self.active.remove(pos);
        if tx.aborted || !commit {
            tx.active = false;
            if tx.mode == Mode::VersionChange {
                self.close(id)?;
            }
            return Err(Error::Abort);
        }
        if !tx.active {
            return Err(Error::TransactionInactive);
        }
        if tx.mode != Mode::ReadOnly {
            let mut db = self
                .databases
                .get(&(connection.key.clone(), connection.name.clone()))
                .ok_or(Error::NotFound)?
                .clone();
            if tx.mode == Mode::VersionChange {
                db = tx.working;
            } else {
                for name in &tx.scope {
                    if let Some(store) = tx.working.stores.get(name) {
                        db.stores.insert(name.clone(), store.clone());
                    }
                }
            }
            self.persist(&connection.key, &connection.name, &db)?;
            self.databases.insert((connection.key, connection.name), db);
            if tx.mode == Mode::VersionChange {
                let version = self
                    .pending_upgrade
                    .remove(&id)
                    .ok_or(Error::InvalidState)?;
                self.events.push_back(OpenEvent::Success {
                    connection: id,
                    version,
                });
            }
        }
        tx.active = false;
        Ok(())
    }
}
