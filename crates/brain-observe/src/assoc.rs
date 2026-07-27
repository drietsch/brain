//! Association: the soft index — *what is related to this?* beyond
//! explicit edges.
//!
//! Deliberately built at the systems-of-query seam: everything here is
//! derived from replaying authoritative objects, disposable, and
//! rebuildable — associative recall never becomes a second source of
//! truth (see ADR-009). Signals are deterministic, no embeddings:
//! co-change (files observed changing in the same refresh batch),
//! co-mention (one document naming both), and shared import neighbors.

use brain_core::ids::StableId;
use brain_core::object::Object;
use brain_index::{Index, MemIndex};
use brain_store::{Store, StoreError};
use std::collections::{BTreeMap, BTreeSet};

/// Refresh batches touching more files than this carry little signal
/// (initial ingests relate everything to everything): ignored.
const MAX_BATCH: usize = 8;

pub struct AssocIndex {
    labels: BTreeMap<StableId, String>,
    /// content_b3 observation timestamps -> files changed in that batch.
    batches: BTreeMap<u64, Vec<StableId>>,
    /// doc sid -> the file sids its text mentions.
    doc_mentions: BTreeMap<StableId, Vec<StableId>>,
    /// undirected import neighborhood per file.
    neighbors: BTreeMap<StableId, BTreeSet<StableId>>,
}

impl AssocIndex {
    /// Build from the authoritative graph. Cheap enough to rebuild per
    /// query; owning no truth, it can be thrown away freely.
    pub fn build(store: &Store, index: &MemIndex, prefix: &str) -> Result<AssocIndex, StoreError> {
        let mut a = AssocIndex {
            labels: BTreeMap::new(),
            batches: BTreeMap::new(),
            doc_mentions: BTreeMap::new(),
            neighbors: BTreeMap::new(),
        };
        let ns = store.namespace()?;
        for (name, node) in &ns {
            let Some(rel) = name.strip_prefix(&format!("{prefix}/")) else { continue };
            let Ok(Object::Entity { id: sid, entity_kind, .. }) = store.get(node) else {
                continue;
            };
            if entity_kind != "source_file" {
                continue;
            }
            a.labels.insert(sid.clone(), rel.to_string());
            for oid in index.observations_of(&sid) {
                if let Object::Observation { property, observed_at_ms, .. } = store.get(&oid)? {
                    if property == "content_b3" {
                        a.batches.entry(observed_at_ms).or_default().push(sid.clone());
                    }
                }
            }
            for (_, to) in crate::twin::live_from(index, store, &sid, "imports")? {
                a.neighbors.entry(sid.clone()).or_default().insert(to.clone());
                a.neighbors.entry(to).or_default().insert(sid.clone());
            }
        }
        // Documents of every kind that carries mentions.
        let doc_kinds = crate::kinds::doc_kinds(store, index)?;
        for kind in &doc_kinds {
            let mut seen = BTreeSet::new();
            for node in index.entities_by_kind(kind) {
                let Ok(Object::Entity { id, labels, .. }) = store.get(&node) else { continue };
                if labels.get("prefix").map(String::as_str) != Some(prefix)
                    || !seen.insert(id.clone())
                {
                    continue;
                }
                let slug = labels.get("slug").cloned().unwrap_or_default();
                a.labels.insert(id.clone(), slug);
                let mut targets = Vec::new();
                for (_, to) in crate::twin::live_from(index, store, &id, "mentions")? {
                    targets.push(to);
                }
                if !targets.is_empty() {
                    a.doc_mentions.insert(id.clone(), targets);
                }
            }
        }
        Ok(a)
    }

    /// Rank entities associated with `sid`, strongest first:
    /// (label, score, reasons).
    pub fn related(&self, sid: &StableId) -> Vec<(String, u32, Vec<String>)> {
        let mut scores: BTreeMap<StableId, (u32, Vec<String>)> = BTreeMap::new();

        // Co-change: same refresh batch, small batches only.
        let mut co: BTreeMap<StableId, u32> = BTreeMap::new();
        for files in self.batches.values() {
            if files.len() < 2 || files.len() > MAX_BATCH || !files.contains(sid) {
                continue;
            }
            for other in files {
                if other != sid {
                    *co.entry(other.clone()).or_default() += 1;
                }
            }
        }
        for (other, n) in co {
            let e = scores.entry(other).or_default();
            e.0 += n * 3;
            e.1.push(format!("changed together {n}×"));
        }

        // Co-mention: one document names both.
        for (doc, targets) in &self.doc_mentions {
            if !targets.contains(sid) {
                continue;
            }
            let doc_label =
                self.labels.get(doc).cloned().unwrap_or_else(|| doc.to_string());
            for other in targets {
                if other != sid {
                    let e = scores.entry(other.clone()).or_default();
                    e.0 += 2;
                    e.1.push(format!("both mentioned by {doc_label}"));
                }
            }
        }

        // Shared import neighborhood.
        if let Some(mine) = self.neighbors.get(sid) {
            for (other, theirs) in &self.neighbors {
                if other == sid {
                    continue;
                }
                let shared = mine.intersection(theirs).count() as u32;
                if shared > 0 {
                    let e = scores.entry(other.clone()).or_default();
                    e.0 += shared;
                    e.1.push(format!("share {shared} import neighbor(s)"));
                }
            }
        }

        let mut out: Vec<(String, u32, Vec<String>)> = scores
            .into_iter()
            .filter_map(|(other, (score, reasons))| {
                self.labels.get(&other).map(|l| (l.clone(), score, reasons))
            })
            .collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::twin::refresh;
    use brain_index::replay;
    use std::fs;

    #[test]
    fn co_change_and_co_mention_produce_ranked_associations() {
        let src = tempfile::tempdir().unwrap();
        fs::create_dir_all(src.path().join("src")).unwrap();
        fs::create_dir_all(src.path().join("docs/adr")).unwrap();
        fs::write(src.path().join("src/a.rs"), "pub fn a() {}\n").unwrap();
        fs::write(src.path().join("src/b.rs"), "pub fn b() {}\n").unwrap();
        fs::write(src.path().join("src/c.rs"), "pub fn c() {}\n").unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let store = Store::open(store_dir.path()).unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();

        // a.rs and b.rs repeatedly change together; c.rs stays put.
        for i in 0..3 {
            std::thread::sleep(std::time::Duration::from_millis(2));
            fs::write(src.path().join("src/a.rs"), format!("pub fn a() {{ /* {i} */ }}\n"))
                .unwrap();
            fs::write(src.path().join("src/b.rs"), format!("pub fn b() {{ /* {i} */ }}\n"))
                .unwrap();
            refresh(&store, src.path(), "twin/app").unwrap();
        }
        // One decision mentions both a.rs and c.rs.
        fs::write(
            src.path().join("docs/adr/adr-001-pair.md"),
            "# Pair\n\nStatus: accepted\n\nsrc/a.rs and src/c.rs share a contract.\n",
        )
        .unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();

        let mut index = MemIndex::new();
        replay(&store, &mut index).unwrap();
        let assoc = AssocIndex::build(&store, &index, "twin/app").unwrap();
        let a = StableId::derive(&["file", "src/a.rs"]);
        let related = assoc.related(&a);
        assert!(!related.is_empty());
        // b.rs leads: initial ingest + 3 edit rounds = 4 batches (4×3=12),
        // far ahead of c.rs (initial batch 3 + co-mention 2 = 5).
        assert_eq!(related[0].0, "src/b.rs", "{related:?}");
        assert_eq!(related[0].1, 12);
        assert!(related[0].2.iter().any(|r| r.contains("changed together 4×")));
        let c = related.iter().find(|(l, _, _)| l == "src/c.rs").expect("c.rs related");
        assert_eq!(c.1, 5);
        assert!(c.2.iter().any(|r| r.contains("both mentioned by adr-001-pair")));

        // Disposable and deterministic: a rebuild ranks identically.
        let rebuilt = AssocIndex::build(&store, &index, "twin/app").unwrap();
        assert_eq!(rebuilt.related(&a), related);
    }
}
