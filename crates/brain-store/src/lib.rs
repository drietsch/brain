//! brain-store: the persistence fabric.
//!
//! One content-addressed graph, physically laid out as:
//!
//! ```text
//! <root>/
//!   objects/<h[0..2]>/<h[2..64]>.json   immutable objects, canonical bytes
//!   events.jsonl                        append-only history of graph mutations
//!   intents.jsonl                       durable intent/receipt state (see intents.rs)
//!   HEAD                                NodeId of the current Namespace object
//! ```
//!
//! Files exist here only BELOW the semantic line — the way a database keeps
//! pages on disk. No unit of meaning, editing, versioning or deployment is a
//! file; the graph is authoritative.
//!
//! "Version control" is the Namespace lineage chain: binding a name writes a
//! new Namespace object whose `parent` is the previous HEAD. Definitions are
//! never edited in place, so the codebase can never be broken by a change.

pub mod intents;
pub mod sync;

use brain_core::ids::NodeId;
use brain_core::object::{hash_object, object_bytes, Object};
use brain_core::CoreError;
use serde_json::json;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("core: {0}")]
    Core(#[from] CoreError),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("object {0} not found")]
    NotFound(NodeId),
    #[error("object {id} is corrupt: stored bytes hash to {actual}")]
    Corrupt { id: NodeId, actual: NodeId },
    #[error("expected {expected} object at {id}")]
    WrongKind { id: NodeId, expected: &'static str },
    #[error(
        "canonicalization mismatch during sync: {claimed} now canonicalizes to {actual} — \
             the source store predates the current canonical form; rebuild or migrate it"
    )]
    CanonEpoch { claimed: NodeId, actual: NodeId },
}

/// The put feed, parsed once and held behind the log's byte length.
///
/// `events.jsonl` is append-only, so its length is a sound cursor: a
/// different length means lines were added at the end, never that earlier
/// ones changed. Nothing else in the file can move.
struct HistoryMemo {
    len: u64,
    ids: Arc<Vec<NodeId>>,
    /// Where each object sits in the feed. Insertion order is semantic —
    /// two notes written in the same millisecond keep their true order —
    /// so callers that need ordering ask this instead of walking the log.
    position: Arc<HashMap<NodeId, usize>>,
}

pub struct Store {
    root: PathBuf,
    history: RwLock<Option<HistoryMemo>>,
    /// Objects already read, by identity.
    ///
    /// This cache is correct by construction and needs no invalidation:
    /// an object's id *is* the hash of its bytes, so the bytes behind an
    /// id can never change. Nothing can go stale.
    ///
    /// It earns its place because reads repeat heavily: one `insights`
    /// pass made 19,825 reads over only 4,517 distinct objects, fetching
    /// the hottest 64 times each.
    ///
    /// The integrity property this preserves, stated exactly: **every
    /// object is verified against its id when it enters the process**, and
    /// only then. Entries come from `get`, never from `put`, so the store
    /// never vouches for bytes it did not read back. A file that rots
    /// while a process runs is caught by the next one — which is the same
    /// guarantee any page cache gives, and strictly more than re-hashing
    /// the same bytes 64 times per command bought.
    objects: RwLock<HashMap<NodeId, Arc<Object>>>,
    /// The pack, loaded lazily on the first miss.
    pack: RwLock<Option<Arc<Pack>>>,
    /// Reads served, and reads that had to touch a file. The gap between
    /// them is what the caches are worth, and a test asserts a budget so
    /// a regression shows up as a failure rather than as a feeling.
    reads: AtomicUsize,
    misses: AtomicUsize,
}

/// What the store did to answer, since it was opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reads {
    /// Calls to `get`.
    pub served: usize,
    /// Of those, the ones that read bytes rather than answering from
    /// memory — from the pack or a loose file.
    pub from_disk: usize,
}

/// Every object's bytes in one file, so reading the graph is one
/// sequential read instead of ten thousand.
///
/// This is a **derived, disposable read cache** — the same bargain
/// `cortex` makes for the index. The loose objects under `objects/`
/// remain the system of record; ADR-011 is explicit that the record
/// layer must not move. Delete the pack and it rebuilds; it is never
/// replicated, and nothing authoritative lives here.
///
/// Why it exists: the same 2.6 MB takes 979 ms to read as 10,575 files
/// and 4 ms as one. That is the whole difference, and it is the floor
/// under every command that has to see the graph.
///
/// Layout, repeated until EOF, in put order:
///
/// ```text
/// [32-byte NodeId][u32 LE byte length][canonical bytes]
/// ```
///
/// Self-describing on purpose: the ids are in the record headers, so the
/// pack can be scanned without consulting the event log and a truncated
/// tail costs only the records after the tear.
struct Pack {
    bytes: Vec<u8>,
    /// id -> (start, len) of the canonical bytes within `bytes`.
    at: HashMap<NodeId, (usize, usize)>,
}

const PACK_HEADER: usize = 32 + 4;

impl Pack {
    fn load(path: &Path) -> Option<Pack> {
        let bytes = fs::read(path).ok()?;
        let mut at = HashMap::new();
        let mut cursor = 0usize;
        while cursor + PACK_HEADER <= bytes.len() {
            let mut raw = [0u8; 32];
            raw.copy_from_slice(&bytes[cursor..cursor + 32]);
            let len = u32::from_le_bytes([
                bytes[cursor + 32],
                bytes[cursor + 33],
                bytes[cursor + 34],
                bytes[cursor + 35],
            ]) as usize;
            let start = cursor + PACK_HEADER;
            if start + len > bytes.len() {
                break; // torn tail: everything before it is still good
            }
            at.insert(NodeId(raw), (start, len));
            cursor = start + len;
        }
        Some(Pack { bytes, at })
    }

    fn get(&self, id: &NodeId) -> Option<&[u8]> {
        let (start, len) = self.at.get(id)?;
        Some(&self.bytes[*start..*start + *len])
    }
}

impl Store {
    /// Open (creating if necessary) a store rooted at `root`.
    pub fn open(root: impl AsRef<Path>) -> Result<Store, StoreError> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(root.join("objects"))?;
        Ok(Store {
            root,
            history: RwLock::new(None),
            objects: RwLock::new(HashMap::new()),
            pack: RwLock::new(None),
            reads: AtomicUsize::new(0),
            misses: AtomicUsize::new(0),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn object_path(&self, id: &NodeId) -> PathBuf {
        let hex = id.to_hex();
        self.root
            .join("objects")
            .join(&hex[..2])
            .join(format!("{}.json", &hex[2..]))
    }

    /// Store an object. Idempotent: identical content lands at the identical
    /// path, so a re-put of existing content is a no-op (structural dedup).
    /// Code objects are alpha-normalized on the way in, so alpha-equivalent
    /// programs deduplicate to one node and stored bytes always re-hash to
    /// their id ("identity before names").
    pub fn put(&self, o: &Object) -> Result<NodeId, StoreError> {
        let o = &brain_core::object::canonicalize(o);
        let id = hash_object(o)?;
        let path = self.object_path(&id);
        if !path.exists() {
            fs::create_dir_all(path.parent().expect("object path has parent"))?;
            let bytes = object_bytes(o)?;
            // Write via temp file + rename so a crash never leaves a torn
            // object at a content-addressed path.
            let tmp = path.with_extension("tmp");
            fs::write(&tmp, &bytes)?;
            fs::rename(&tmp, &path)?;
            self.append_event("put", json!({ "id": id.to_string() }))?;
        }
        // Deliberately not cached here. The cache holds only what was read
        // back from disk and verified against its id; seeding it from the
        // value in hand would mean the store vouches for bytes it never
        // checked, and `corrupt_object_is_detected` is the test that says
        // so. A writer re-reading what it wrote pays one verified read.
        Ok(id)
    }

    pub fn get(&self, id: &NodeId) -> Result<Object, StoreError> {
        Ok(self.get_shared(id)?.as_ref().clone())
    }

    /// The same object without copying it.
    ///
    /// Prefer this on hot paths: `get` clones, and most callers only read
    /// a field or two before dropping the result.
    pub fn get_shared(&self, id: &NodeId) -> Result<Arc<Object>, StoreError> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        if let Ok(cache) = self.objects.read() {
            if let Some(hit) = cache.get(id) {
                return Ok(hit.clone());
            }
        }
        // The pack first: one file already open beats one file per object.
        // Its bytes are verified exactly as a loose object's are, so a
        // damaged pack is caught rather than trusted.
        if let Some(pack) = self.pack() {
            if let Some(bytes) = pack.get(id) {
                return self.admit(id, bytes);
            }
        }

        let path = self.object_path(id);
        let bytes = fs::read(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                StoreError::NotFound(*id)
            } else {
                StoreError::Io(e)
            }
        })?;
        self.admit(id, &bytes)
    }

    /// Verify bytes against the id they claim, parse them, and remember.
    fn admit(&self, id: &NodeId, bytes: &[u8]) -> Result<Arc<Object>, StoreError> {
        self.misses.fetch_add(1, Ordering::Relaxed);
        // Integrity check: recompute content identity from stored bytes.
        // Done once, on the way into the cache — the bytes behind an id
        // cannot change, so verifying a second time proves nothing new.
        let actual = brain_core::canonical::hash_bytes(bytes);
        if actual != *id {
            return Err(StoreError::Corrupt { id: *id, actual });
        }
        let object = Arc::new(serde_json::from_slice::<Object>(bytes)?);
        if let Ok(mut cache) = self.objects.write() {
            cache.insert(*id, object.clone());
        }
        Ok(object)
    }

    fn pack_path(&self) -> PathBuf {
        self.root.join("objects.pack")
    }

    fn pack(&self) -> Option<Arc<Pack>> {
        if let Ok(guard) = self.pack.read() {
            if let Some(pack) = guard.as_ref() {
                return Some(pack.clone());
            }
        }
        let loaded = Arc::new(Pack::load(&self.pack_path())?);
        if let Ok(mut guard) = self.pack.write() {
            *guard = Some(loaded.clone());
        }
        Some(loaded)
    }

    /// Append to the pack whatever the loose store has and it does not.
    ///
    /// The same bargain `cortex.checkpoint()` makes: best-effort, cheap in
    /// the steady state (only new objects are copied), and safe to delete
    /// — the loose objects remain the record. Returns how many were added.
    pub fn compact(&self) -> Result<usize, StoreError> {
        let existing: Option<Arc<Pack>> = self.pack();
        let mut appended = 0usize;
        let mut out: Vec<u8> = Vec::new();
        for id in self.put_history_shared()?.iter() {
            if existing.as_ref().is_some_and(|p| p.at.contains_key(id)) {
                continue;
            }
            let path = self.object_path(id);
            let Ok(bytes) = fs::read(&path) else { continue };
            out.extend_from_slice(&id.0);
            out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            out.extend_from_slice(&bytes);
            appended += 1;
        }
        if appended == 0 {
            return Ok(0);
        }
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.pack_path())?;
        f.write_all(&out)?;
        // Force a reload: the file on disk is now longer than what is held.
        if let Ok(mut guard) = self.pack.write() {
            *guard = None;
        }
        Ok(appended)
    }

    /// How many reads this store served, and how many it had to go to
    /// bytes for.
    pub fn reads(&self) -> Reads {
        Reads {
            served: self.reads.load(Ordering::Relaxed),
            from_disk: self.misses.load(Ordering::Relaxed),
        }
    }

    pub fn has(&self, id: &NodeId) -> bool {
        self.object_path(id).exists()
    }

    pub fn count_objects(&self) -> Result<usize, StoreError> {
        let mut n = 0;
        let objects = self.root.join("objects");
        for shard in fs::read_dir(&objects)? {
            let shard = shard?;
            if shard.file_type()?.is_dir() {
                n += fs::read_dir(shard.path())?.count();
            }
        }
        Ok(n)
    }

    // ---- namespace layer: the graph as a codebase ----

    fn head_path(&self) -> PathBuf {
        self.root.join("HEAD")
    }

    /// NodeId of the current Namespace object, if any binding has ever happened.
    pub fn head(&self) -> Result<Option<NodeId>, StoreError> {
        let path = self.head_path();
        if !path.exists() {
            return Ok(None);
        }
        let s = fs::read_to_string(path)?;
        Ok(Some(NodeId::parse(s.trim())?))
    }

    /// Current name -> node bindings (empty map before the first bind).
    pub fn namespace(&self) -> Result<BTreeMap<String, NodeId>, StoreError> {
        match self.head()? {
            None => Ok(BTreeMap::new()),
            Some(id) => match self.get(&id)? {
                Object::Namespace { entries, .. } => Ok(entries),
                _ => Err(StoreError::WrongKind {
                    id,
                    expected: "namespace",
                }),
            },
        }
    }

    pub fn resolve(&self, name: &str) -> Result<Option<NodeId>, StoreError> {
        Ok(self.namespace()?.get(name).copied())
    }

    pub fn bind(&self, name: &str, target: NodeId) -> Result<NodeId, StoreError> {
        self.bind_many(vec![(name.to_string(), target)])
    }

    /// Bind several names in one namespace step (one lineage entry).
    pub fn bind_many(&self, pairs: Vec<(String, NodeId)>) -> Result<NodeId, StoreError> {
        let parent = self.head()?;
        let mut entries = self.namespace()?;
        let names: Vec<String> = pairs.iter().map(|(n, _)| n.clone()).collect();
        for (name, target) in pairs {
            entries.insert(name, target);
        }
        let ns = Object::Namespace { entries, parent };
        let id = self.put(&ns)?;
        fs::write(self.head_path(), id.to_string())?;
        self.append_event(
            "bind",
            json!({ "names": names, "namespace": id.to_string() }),
        )?;
        Ok(id)
    }

    /// Namespace lineage from HEAD backwards (most recent first).
    pub fn namespace_history(&self) -> Result<Vec<NodeId>, StoreError> {
        let mut out = Vec::new();
        let mut cursor = self.head()?;
        while let Some(id) = cursor {
            out.push(id);
            cursor = match self.get(&id)? {
                Object::Namespace { parent, .. } => parent,
                _ => None,
            };
        }
        Ok(out)
    }

    /// All object ids ever put, in event-log order. This is the replay feed
    /// for derived indexes: a system of query is rebuilt from this history,
    /// never treated as a second system of record.
    pub fn put_history(&self) -> Result<Vec<NodeId>, StoreError> {
        Ok(self.put_history_shared()?.as_ref().clone())
    }

    /// The same feed without copying it.
    ///
    /// Parsing the log is not cheap — it was being redone for every caller,
    /// and `notes()` asked once per subject, so a single `insights` pass
    /// re-read and re-parsed the whole log 195 times. The log only ever
    /// grows, so its byte length says exactly when the parse is still good.
    pub fn put_history_shared(&self) -> Result<Arc<Vec<NodeId>>, StoreError> {
        Ok(self.history_memo()?.0)
    }

    /// Where each object sits in the put feed.
    ///
    /// Insertion order is semantic: the log is chronological by
    /// construction, so two observations written in the same millisecond
    /// keep their true order. Callers that need that order look a position
    /// up here rather than walking the whole log to find it.
    pub fn put_position(&self) -> Result<Arc<HashMap<NodeId, usize>>, StoreError> {
        Ok(self.history_memo()?.1)
    }

    fn history_memo(&self) -> Result<(Arc<Vec<NodeId>>, Arc<HashMap<NodeId, usize>>), StoreError> {
        let path = self.root.join("events.jsonl");
        let len = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);

        if let Ok(guard) = self.history.read() {
            if let Some(memo) = guard.as_ref() {
                if memo.len == len {
                    return Ok((memo.ids.clone(), memo.position.clone()));
                }
            }
        }

        let mut ids = Vec::new();
        if path.exists() {
            for line in fs::read_to_string(&path)?.lines() {
                if line.trim().is_empty() {
                    continue;
                }
                let v: serde_json::Value = serde_json::from_str(line)?;
                if v.get("kind").and_then(|k| k.as_str()) == Some("put") {
                    if let Some(id) = v
                        .get("detail")
                        .and_then(|d| d.get("id"))
                        .and_then(|i| i.as_str())
                    {
                        ids.push(NodeId::parse(id)?);
                    }
                }
            }
        }
        let position: HashMap<NodeId, usize> = ids
            .iter()
            .enumerate()
            .map(|(at, id)| (*id, at))
            .collect();

        let ids = Arc::new(ids);
        let position = Arc::new(position);
        if let Ok(mut guard) = self.history.write() {
            *guard = Some(HistoryMemo {
                len,
                ids: ids.clone(),
                position: position.clone(),
            });
        }
        Ok((ids, position))
    }

    // ---- event log ----

    pub(crate) fn append_event(
        &self,
        kind: &str,
        detail: serde_json::Value,
    ) -> Result<(), StoreError> {
        let line = json!({ "at_ms": now_ms(), "kind": kind, "detail": detail });
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.root.join("events.jsonl"))?;
        writeln!(f, "{line}")?;
        Ok(())
    }

    /// The durable intent log for this store.
    pub fn intents(&self) -> intents::IntentLog {
        intents::IntentLog::new(self.root.join("intents.jsonl"))
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use brain_core::object::{Literal, Term};

    fn code(i: i64) -> Object {
        Object::Code {
            term: Term::Lit {
                value: Literal::Int { value: i },
            },
        }
    }

    #[test]
    fn put_get_roundtrip_and_dedup() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let id1 = store.put(&code(42)).unwrap();
        let id2 = store.put(&code(42)).unwrap();
        assert_eq!(id1, id2);
        assert_eq!(store.count_objects().unwrap(), 1);
        assert_eq!(store.get(&id1).unwrap(), code(42));
    }

    #[test]
    fn alpha_equivalent_programs_deduplicate() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let identity = |p: &str| Object::Code {
            term: Term::Lam {
                param: p.to_string(),
                body: Box::new(Term::Var {
                    name: p.to_string(),
                }),
            },
        };
        let a = store.put(&identity("x")).unwrap();
        let b = store.put(&identity("y")).unwrap();
        assert_eq!(a, b);
        assert_eq!(store.count_objects().unwrap(), 1);
        // Stored bytes re-hash to the id (integrity holds post-normalization).
        assert!(store.get(&a).is_ok());
    }

    #[test]
    fn missing_object_is_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let id = hash_object(&code(1)).unwrap();
        assert!(matches!(store.get(&id), Err(StoreError::NotFound(_))));
    }

    #[test]
    fn corrupt_object_is_detected() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let id = store.put(&code(7)).unwrap();
        fs::write(store.object_path(&id), b"{\"kind\":\"tampered\"}").unwrap();
        assert!(matches!(store.get(&id), Err(StoreError::Corrupt { .. })));
    }

    /// The object cache must never vouch for bytes the store did not read
    /// back and verify. A `put` therefore does not seed it — otherwise a
    /// writer's own value would shadow whatever is actually on disk, and
    /// corruption would go unnoticed for the life of the process.
    #[test]
    fn the_cache_holds_only_what_was_verified_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let id = store.put(&code(7)).unwrap();

        // Never read, so never cached: tampering is caught on first read.
        fs::write(store.object_path(&id), b"{\"kind\":\"tampered\"}").unwrap();
        assert!(matches!(store.get(&id), Err(StoreError::Corrupt { .. })));

        // And a verified read is what populates the cache, so a second
        // read of a good object agrees with the first.
        let good = store.put(&code(9)).unwrap();
        let first = store.get(&good).unwrap();
        let second = store.get(&good).unwrap();
        assert_eq!(first, second);
    }

    /// The pack is a read cache, and it is held to the same standard as
    /// the loose objects it accelerates: bytes are verified against the id
    /// that claims them. A pack that has been tampered with must be caught,
    /// never trusted because it is convenient.
    #[test]
    fn a_tampered_pack_is_caught_like_a_tampered_object() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let id = store.put(&code(11)).unwrap();
        assert_eq!(store.compact().unwrap(), 1);

        // Rewrite the payload in place, keeping the header intact.
        let mut raw = fs::read(dir.path().join("objects.pack")).unwrap();
        let start = 32 + 4;
        for (i, b) in br#"{"kind":"tampered"} "#.iter().enumerate() {
            if start + i < raw.len() {
                raw[start + i] = *b;
            }
        }
        fs::write(dir.path().join("objects.pack"), &raw).unwrap();

        // A fresh Store, so nothing is answered from the object cache.
        let cold = Store::open(dir.path()).unwrap();
        assert!(matches!(cold.get(&id), Err(StoreError::Corrupt { .. })));
    }

    /// The pack is disposable. Deleting it must cost speed and nothing
    /// else — the loose objects are still the record.
    #[test]
    fn deleting_the_pack_loses_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let ids: Vec<_> = (0..5).map(|n| store.put(&code(n)).unwrap()).collect();
        assert_eq!(store.compact().unwrap(), 5);
        assert_eq!(store.compact().unwrap(), 0, "compaction is idempotent");

        fs::remove_file(dir.path().join("objects.pack")).unwrap();
        let cold = Store::open(dir.path()).unwrap();
        for (n, id) in ids.iter().enumerate() {
            assert_eq!(cold.get(id).unwrap(), code(n as i64));
        }

        // And it rebuilds from the loose objects, to the same answers.
        assert_eq!(cold.compact().unwrap(), 5);
        let again = Store::open(dir.path()).unwrap();
        for (n, id) in ids.iter().enumerate() {
            assert_eq!(again.get(id).unwrap(), code(n as i64));
        }
    }

    /// A pack whose tail was lost to a crash still serves everything
    /// written before the tear, and the loose store covers the rest.
    #[test]
    fn a_torn_pack_serves_what_survived_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let ids: Vec<_> = (0..6).map(|n| store.put(&code(n)).unwrap()).collect();
        store.compact().unwrap();

        let raw = fs::read(dir.path().join("objects.pack")).unwrap();
        fs::write(dir.path().join("objects.pack"), &raw[..raw.len() / 2]).unwrap();

        let cold = Store::open(dir.path()).unwrap();
        for (n, id) in ids.iter().enumerate() {
            assert_eq!(cold.get(id).unwrap(), code(n as i64), "object {n}");
        }
    }

    #[test]
    fn namespace_bindings_carry_lineage() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let a = store.put(&code(1)).unwrap();
        let b = store.put(&code(2)).unwrap();

        store.bind("math/one", a).unwrap();
        store.bind("math/two", b).unwrap();

        assert_eq!(store.resolve("math/one").unwrap(), Some(a));
        assert_eq!(store.resolve("math/two").unwrap(), Some(b));

        // Rebinding a name never edits in place: it adds a lineage step.
        store.bind("math/one", b).unwrap();
        assert_eq!(store.resolve("math/one").unwrap(), Some(b));
        assert_eq!(store.namespace_history().unwrap().len(), 3);
    }
}
