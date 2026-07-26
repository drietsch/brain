//! cortex: brain's own persistent graph-query engine.
//!
//! Learned from minigraf (single-file persistence, recursive graph
//! queries, temporal reads) and then radically simplified by one
//! observation: **brain already has a WAL** — the store's event log. So
//! cortex has no write path of its own. It is a checkpoint of derived
//! index state plus delta-replay from a cursor into `put_history()`:
//!
//! - warm open is O(new events since the last checkpoint), not O(graph);
//! - the `.graf` file is derived, disposable, and rebuildable — corrupt
//!   or missing means a silent cold rebuild, never an error;
//! - it is local by design and never replicates: truth travels as
//!   objects, indexes are grown where they are needed.
//!
//! `Cortex` derefs to [`MemIndex`], so every existing query path works
//! unchanged; on top it adds what a flat index cannot express —
//! transitive reachability over relation edges.

use brain_core::ids::{NodeId, StableId};
use brain_core::object::Object;
use brain_index::{Index, IndexSnapshot, MemIndex};
use brain_store::{Store, StoreError};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, VecDeque};
use std::fs;
use std::ops::Deref;
use std::path::PathBuf;

/// Bump when replay semantics change: a version mismatch is just a cold
/// rebuild, exactly like a missing file.
const FORMAT_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct CortexFile {
    version: u32,
    /// Number of event-log entries already folded into the snapshot.
    cursor: usize,
    snapshot: IndexSnapshot,
}

pub struct Cortex {
    index: MemIndex,
    cursor: usize,
    path: PathBuf,
    /// Cursor value the loaded checkpoint had (to skip no-op writes).
    checkpointed: usize,
}

impl Deref for Cortex {
    type Target = MemIndex;
    fn deref(&self) -> &MemIndex {
        &self.index
    }
}

impl Cortex {
    /// Open the persistent index for a store: load the checkpoint if one
    /// is usable, then catch up on the event-log delta.
    pub fn open(store: &Store) -> Result<Cortex, StoreError> {
        let path = store.root().join("cortex.json");
        let (mut index, mut cursor) = match Self::load(&path) {
            Some((idx, cur)) => (idx, cur),
            None => (MemIndex::new(), 0),
        };
        let checkpointed = cursor;
        let history = store.put_history()?;
        // A shrunken or diverged log (rebuilt store) invalidates the
        // checkpoint: cold rebuild, silently — disposability is the contract.
        if cursor > history.len() {
            index = MemIndex::new();
            cursor = 0;
        }
        for id in &history[cursor..] {
            let obj = store.get(id)?;
            index.on_object(id, &obj);
        }
        cursor = history.len();
        Ok(Cortex { index, cursor, path, checkpointed })
    }

    /// A cold, non-persisting build: the reference behavior, for
    /// benchmarking and `BRAIN_INDEX=mem` paranoia. `checkpoint()` on an
    /// ephemeral Cortex is a no-op.
    pub fn open_ephemeral(store: &Store) -> Result<Cortex, StoreError> {
        let mut index = MemIndex::new();
        let history = store.put_history()?;
        for id in &history {
            index.on_object(id, &store.get(id)?);
        }
        let cursor = history.len();
        Ok(Cortex { index, cursor, path: store.root().join("cortex.json"), checkpointed: cursor })
    }

    fn load(path: &PathBuf) -> Option<(MemIndex, usize)> {
        let bytes = fs::read(path).ok()?;
        let file: CortexFile = serde_json::from_slice(&bytes).ok()?;
        if file.version != FORMAT_VERSION {
            return None;
        }
        Some((MemIndex::restore(file.snapshot), file.cursor))
    }

    /// Persist the current state (temp + rename). A no-op when nothing
    /// new was folded in since the loaded checkpoint.
    pub fn checkpoint(&self) -> Result<(), StoreError> {
        if self.cursor == self.checkpointed {
            return Ok(());
        }
        let file = CortexFile {
            version: FORMAT_VERSION,
            cursor: self.cursor,
            snapshot: self.index.snapshot(),
        };
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_vec(&file)?)?;
        fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    /// Events folded in beyond the loaded checkpoint (0 on a warm open
    /// with nothing new).
    pub fn delta(&self) -> usize {
        self.cursor - self.checkpointed.min(self.cursor)
    }

    /// Transitive reachability over `predicate` relations: BFS from
    /// `from`, following edges forward (`reverse=false`: what does this
    /// reach?) or backward (`reverse=true`: what reaches this? — the
    /// blast radius). Cycle-safe; returns (entity, depth), nearest first.
    pub fn reach(
        &self,
        store: &Store,
        from: &StableId,
        predicate: &str,
        reverse: bool,
        max_depth: usize,
    ) -> Result<Vec<(StableId, usize)>, StoreError> {
        let mut seen: BTreeSet<StableId> = BTreeSet::new();
        seen.insert(from.clone());
        let mut out = Vec::new();
        let mut queue: VecDeque<(StableId, usize)> = VecDeque::new();
        queue.push_back((from.clone(), 0));
        while let Some((sid, depth)) = queue.pop_front() {
            if depth >= max_depth {
                continue;
            }
            let edges = if reverse {
                self.index.relations_to(&sid, predicate)
            } else {
                self.index.relations_from(&sid, predicate)
            };
            for id in edges {
                if let Object::Relation { from: f, to: t, .. } = store.get(&id)? {
                    let next = if reverse { f } else { t };
                    if seen.insert(next.clone()) {
                        out.push((next.clone(), depth + 1));
                        queue.push_back((next, depth + 1));
                    }
                }
            }
        }
        Ok(out)
    }
}

/// Compare two indexes across the whole Index API for a set of probes —
/// the correctness harness behind "cortex answers exactly like the
/// reference backend".
pub fn answers_match(
    a: &dyn Index,
    b: &dyn Index,
    nodes: &[NodeId],
    sids: &[StableId],
    kinds: &[&str],
    predicates: &[&str],
) -> bool {
    for n in nodes {
        if a.referrers(n) != b.referrers(n)
            || a.evidence_for(n) != b.evidence_for(n)
            || a.receipts_for(n) != b.receipts_for(n)
        {
            return false;
        }
    }
    for s in sids {
        if a.observations_of(s) != b.observations_of(s) || a.entity_nodes(s) != b.entity_nodes(s) {
            return false;
        }
        for p in predicates {
            if a.relations_from(s, p) != b.relations_from(s, p)
                || a.relations_to(s, p) != b.relations_to(s, p)
            {
                return false;
            }
        }
    }
    kinds.iter().all(|k| a.entities_by_kind(k) == b.entities_by_kind(k))
}

#[cfg(test)]
mod tests {
    use super::*;
    use brain_index::replay;
    use std::collections::BTreeMap;

    fn populate(store: &Store, n: usize) -> (Vec<StableId>, Vec<NodeId>) {
        let mut sids: Vec<StableId> = Vec::new();
        let mut nodes: Vec<NodeId> = Vec::new();
        for i in 0..n {
            let sid = StableId::derive(&["file", &format!("src/f{i}.rs")]);
            let node = store
                .put(&Object::Entity {
                    id: sid.clone(),
                    entity_kind: "source_file".to_string(),
                    labels: BTreeMap::new(),
                })
                .unwrap();
            store
                .put(&Object::Observation {
                    subject: sid.clone(),
                    property: "content_b3".to_string(),
                    value: format!("hash{i}"),
                    source: "twin".to_string(),
                    observed_at_ms: i as u64,
                })
                .unwrap();
            if i > 0 {
                store
                    .put(&Object::Relation {
                        from: sids[i - 1].clone(),
                        predicate: "imports".to_string(),
                        to: sid.clone(),
                        source: "twin".to_string(),
                        observed_at_ms: i as u64,
                    })
                    .unwrap();
            }
            sids.push(sid);
            nodes.push(node);
        }
        (sids, nodes)
    }

    #[test]
    fn warm_open_answers_exactly_like_cold_rebuild() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let (sids, nodes) = populate(&store, 10);

        // Cold open + checkpoint, then a second (warm) open.
        let cold = Cortex::open(&store).unwrap();
        assert!(cold.delta() > 0);
        cold.checkpoint().unwrap();
        let warm = Cortex::open(&store).unwrap();
        assert_eq!(warm.delta(), 0, "nothing new after checkpoint");

        let mut reference = MemIndex::new();
        replay(&store, &mut reference).unwrap();
        assert!(answers_match(
            &*warm,
            &reference,
            &nodes,
            &sids,
            &["source_file", "template"],
            &["imports", "contains"],
        ));
    }

    #[test]
    fn cursor_catches_up_and_corrupt_checkpoint_rebuilds() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let (mut sids, mut nodes) = populate(&store, 5);
        Cortex::open(&store).unwrap().checkpoint().unwrap();

        // New objects after the checkpoint are seen via delta replay.
        let (more_sids, more_nodes) = {
            let sid = StableId::derive(&["file", "src/late.rs"]);
            let node = store
                .put(&Object::Entity {
                    id: sid.clone(),
                    entity_kind: "source_file".to_string(),
                    labels: BTreeMap::new(),
                })
                .unwrap();
            (vec![sid], vec![node])
        };
        sids.extend(more_sids.clone());
        nodes.extend(more_nodes);
        let warm = Cortex::open(&store).unwrap();
        assert!(warm.delta() > 0, "delta replay saw the new objects");
        assert_eq!(warm.entity_nodes(&more_sids[0]).len(), 1);

        // Corrupting the checkpoint is not an error — just a cold rebuild.
        fs::write(dir.path().join("cortex.json"), b"not json at all").unwrap();
        let rebuilt = Cortex::open(&store).unwrap();
        let mut reference = MemIndex::new();
        replay(&store, &mut reference).unwrap();
        assert!(answers_match(&*rebuilt, &reference, &nodes, &sids, &["source_file"], &["imports"]));
    }

    #[test]
    fn reach_walks_transitively_both_ways_and_survives_cycles() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let (sids, _) = populate(&store, 4); // chain f0 -> f1 -> f2 -> f3
        // Add a cycle back edge f3 -> f0.
        store
            .put(&Object::Relation {
                from: sids[3].clone(),
                predicate: "imports".to_string(),
                to: sids[0].clone(),
                source: "twin".to_string(),
                observed_at_ms: 99,
            })
            .unwrap();
        let graf = Cortex::open(&store).unwrap();

        // Forward: everything f0 transitively imports, with depths.
        let fwd = graf.reach(&store, &sids[0], "imports", false, 10).unwrap();
        assert_eq!(fwd.len(), 3, "{fwd:?}");
        assert_eq!(fwd[0], (sids[1].clone(), 1));
        assert_eq!(fwd[2], (sids[3].clone(), 3));

        // Reverse from f3: the blast radius of changing it.
        let back = graf.reach(&store, &sids[3], "imports", true, 10).unwrap();
        assert_eq!(back.len(), 3);
        assert_eq!(back[0], (sids[2].clone(), 1));

        // Depth cap respected.
        let capped = graf.reach(&store, &sids[0], "imports", false, 1).unwrap();
        assert_eq!(capped.len(), 1);
    }
}
