//! brain-observe: reflective mode — the graph as a twin of external software.
//!
//! Ingestion walks a source tree and records, per file:
//!
//! - an `Entity` with a *stable* id derived from the relative path (observing
//!   the same file twice yields the same identity — ingestion is idempotent),
//! - an `Observation` of its current content hash (time-bound and sourced:
//!   a claim about the world at a moment, never an eternal truth),
//! - a namespace binding under `<prefix>/<path>` so the twin is navigable.
//!
//! Shared identity is the migration path: when a twinned entity later gains a
//! native implementation, it is the same node acquiring a new edge — not a
//! re-modeling. Observers are sense organs, meant to run continuously;
//! re-ingesting refreshes observations and surfaces drift as new nodes,
//! never overwrites.

use brain_core::ids::StableId;
use brain_core::object::Object;
use brain_store::{now_ms, Store, StoreError};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// File extensions worth twinning in a source tree.
const INGEST_EXTENSIONS: &[&str] = &["rs", "toml", "md", "json"];
/// Directories that are build products or substrate internals, not software.
const SKIP_DIRS: &[&str] = &[".git", "target", ".brain", "node_modules"];

#[derive(Debug, Default, PartialEq)]
pub struct IngestReport {
    pub files: usize,
    pub entities: usize,
    pub observations: usize,
}

/// Ingest a directory tree into the graph as a twin, binding entities under
/// `<prefix>/<relative path>`.
pub fn ingest_dir(store: &Store, root: &Path, prefix: &str) -> Result<IngestReport, StoreError> {
    let mut report = IngestReport::default();
    let mut bindings: Vec<(String, brain_core::ids::NodeId)> = Vec::new();
    let observed_at_ms = now_ms();

    let mut files = Vec::new();
    collect_files(root, root, &mut files)?;
    files.sort();

    for rel in files {
        let full = root.join(&rel);
        let content = fs::read(&full)?;
        report.files += 1;

        let id = StableId::derive(&["file", &rel]);
        let mut labels = BTreeMap::new();
        labels.insert("path".to_string(), rel.clone());
        let entity = Object::Entity {
            id: id.clone(),
            entity_kind: "source_file".to_string(),
            labels,
        };
        let entity_node = store.put(&entity)?;
        report.entities += 1;

        let observation = Object::Observation {
            subject: id,
            property: "content_b3".to_string(),
            value: blake3::hash(&content).to_hex().to_string(),
            source: "brain-observe/ingest".to_string(),
            observed_at_ms,
        };
        store.put(&observation)?;
        report.observations += 1;

        bindings.push((format!("{prefix}/{rel}"), entity_node));
    }

    if !bindings.is_empty() {
        // One namespace step for the whole ingestion: one lineage entry.
        store.bind_many(bindings)?;
    }
    Ok(report)
}

fn collect_files(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), StoreError> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if entry.file_type()?.is_dir() {
            if SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            collect_files(root, &path, out)?;
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if INGEST_EXTENSIONS.contains(&ext) {
                if let Ok(rel) = path.strip_prefix(root) {
                    out.push(rel.to_string_lossy().replace('\\', "/"));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ingest_is_idempotent_in_identity_and_navigable_by_name() {
        let src = tempfile::tempdir().unwrap();
        fs::create_dir_all(src.path().join("src")).unwrap();
        fs::write(src.path().join("src/lib.rs"), "pub fn f() {}").unwrap();
        fs::write(src.path().join("Cargo.toml"), "[package]").unwrap();
        fs::write(src.path().join("ignore.bin"), [0u8, 1]).unwrap();

        let store_dir = tempfile::tempdir().unwrap();
        let store = Store::open(store_dir.path()).unwrap();

        let report = ingest_dir(&store, src.path(), "twin/self").unwrap();
        assert_eq!(
            report,
            IngestReport { files: 2, entities: 2, observations: 2 }
        );

        let entity = store.resolve("twin/self/src/lib.rs").unwrap().unwrap();
        match store.get(&entity).unwrap() {
            Object::Entity { id, entity_kind, .. } => {
                assert_eq!(entity_kind, "source_file");
                assert_eq!(id, StableId::derive(&["file", "src/lib.rs"]));
            }
            other => panic!("expected entity, got {other:?}"),
        }

        // Unchanged content re-ingested: same entities dedup to the same
        // nodes; only fresh observations are added.
        let before = store.count_objects().unwrap();
        ingest_dir(&store, src.path(), "twin/self").unwrap();
        let after = store.count_objects().unwrap();
        // At most: new observation objects (fresh timestamps) + one namespace.
        assert!(after <= before + 3, "unexpected growth: {before} -> {after}");
    }
}
