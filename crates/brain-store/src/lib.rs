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
use std::collections::BTreeMap;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
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

pub struct Store {
    root: PathBuf,
}

impl Store {
    /// Open (creating if necessary) a store rooted at `root`.
    pub fn open(root: impl AsRef<Path>) -> Result<Store, StoreError> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(root.join("objects"))?;
        Ok(Store { root })
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
        Ok(id)
    }

    pub fn get(&self, id: &NodeId) -> Result<Object, StoreError> {
        let path = self.object_path(id);
        let bytes = fs::read(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                StoreError::NotFound(*id)
            } else {
                StoreError::Io(e)
            }
        })?;
        // Integrity check: recompute content identity from stored bytes.
        let actual = brain_core::canonical::hash_bytes(&bytes);
        if actual != *id {
            return Err(StoreError::Corrupt { id: *id, actual });
        }
        Ok(serde_json::from_slice(&bytes)?)
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
        let path = self.root.join("events.jsonl");
        let mut out = Vec::new();
        if !path.exists() {
            return Ok(out);
        }
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
                    out.push(NodeId::parse(id)?);
                }
            }
        }
        Ok(out)
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
