//! brain-index: the seam between the system of record and systems of query.
//!
//! The CAS + event log are authoritative and immutable. Anything that makes
//! them *queryable* — reverse edges, subject lookups, similarity search — is
//! a derived structure: disposable, rebuildable from `Store::put_history()`,
//! and never a second source of truth. If an index corrupts, delete it and
//! replay. If an engine disappoints, swap the [`Index`] implementation.
//!
//! [`MemIndex`] is the naive reference backend: correct, idempotent by
//! construction (set semantics), and the benchmark baseline any embedded
//! graph-database backend (e.g. OverGraph, Graph_D) must beat on real
//! workloads before it earns adoption.

use brain_core::ids::{NodeId, StableId};
use brain_core::object::{Object, Term};
use brain_store::{Store, StoreError};
use std::collections::{BTreeMap, BTreeSet};

// ---------------------------------------------------------------------------
// Edge extraction
// ---------------------------------------------------------------------------

/// Kinds of object-to-object edges that exist in the graph. Edges live
/// *inside* objects; this enumeration is how they become traversable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum EdgeKind {
    /// A `ref` inside a code term: dependency edge, replaces imports/linking.
    CodeRef,
    /// Evidence -> the node it is a claim about.
    EvidenceSubject,
    /// Receipt -> the intent it settles.
    ReceiptIntent,
    /// Namespace -> a bound node.
    NamespaceEntry,
    /// Namespace -> its predecessor (the lineage chain).
    NamespaceParent,
}

/// Extract every object-to-object edge from an object. Pure and total: this
/// is the single place edge semantics are defined, shared by every backend.
/// (`Intent.arg_hash` is deliberately absent: it is a digest of a value, not
/// a reference to an object.)
pub fn object_edges(obj: &Object) -> Vec<(EdgeKind, NodeId)> {
    let mut out = Vec::new();
    match obj {
        Object::Code { term } => {
            let mut refs = Vec::new();
            term_refs(term, &mut refs);
            out.extend(refs.into_iter().map(|id| (EdgeKind::CodeRef, id)));
        }
        Object::Evidence { subject, .. } => out.push((EdgeKind::EvidenceSubject, *subject)),
        Object::Receipt { intent, .. } => out.push((EdgeKind::ReceiptIntent, *intent)),
        Object::Namespace { entries, parent } => {
            out.extend(entries.values().map(|id| (EdgeKind::NamespaceEntry, *id)));
            if let Some(p) = parent {
                out.push((EdgeKind::NamespaceParent, *p));
            }
        }
        Object::Spec { .. }
        | Object::Capability { .. }
        | Object::Entity { .. }
        | Object::Observation { .. }
        | Object::Intent { .. } => {}
    }
    out
}

fn term_refs(term: &Term, out: &mut Vec<NodeId>) {
    match term {
        Term::RefNode { node } => out.push(*node),
        Term::Lit { .. } | Term::Var { .. } | Term::Hole { .. } => {}
        Term::Lam { body, .. } => term_refs(body, out),
        Term::App { func, arg } => {
            term_refs(func, out);
            term_refs(arg, out);
        }
        Term::Let { value, body, .. } => {
            term_refs(value, out);
            term_refs(body, out);
        }
        Term::Record { fields } => {
            for t in fields.values() {
                term_refs(t, out);
            }
        }
        Term::Field { record, .. } => term_refs(record, out),
        Term::Variant { payload, .. } => term_refs(payload, out),
        Term::Match { scrutinee, arms, default } => {
            term_refs(scrutinee, out);
            for arm in arms.values() {
                term_refs(&arm.body, out);
            }
            if let Some(d) = default {
                term_refs(d, out);
            }
        }
        Term::Foreign { arg, .. } => term_refs(arg, out),
    }
}

// ---------------------------------------------------------------------------
// The Index trait: what any backend must provide
// ---------------------------------------------------------------------------

/// A derived query structure over the graph. Implementations MUST be
/// idempotent under repeated `on_object` calls with the same object —
/// replay is the rebuild mechanism and may deliver duplicates.
pub trait Index {
    /// Feed one object (from replay or a live put) into the index.
    fn on_object(&mut self, id: &NodeId, obj: &Object);

    /// Which objects contain an edge to `target`? (Reverse-edge query.)
    fn referrers(&self, target: &NodeId) -> Vec<NodeId>;

    /// All observation nodes about a subject.
    fn observations_of(&self, subject: &StableId) -> Vec<NodeId>;

    /// All entity nodes carrying a stable id (multiple versions may exist).
    fn entity_nodes(&self, id: &StableId) -> Vec<NodeId>;

    /// All entity nodes of a given kind.
    fn entities_by_kind(&self, kind: &str) -> Vec<NodeId>;

    /// All evidence nodes whose claims are about `subject`.
    fn evidence_for(&self, subject: &NodeId) -> Vec<NodeId>;

    /// All receipts settling an intent.
    fn receipts_for(&self, intent: &NodeId) -> Vec<NodeId>;
}

/// Rebuild an index from the store's put history, in event order.
/// Returns the number of objects fed.
pub fn replay(store: &Store, index: &mut dyn Index) -> Result<usize, StoreError> {
    let ids = store.put_history()?;
    let mut fed = 0;
    for id in ids {
        let obj = store.get(&id)?;
        index.on_object(&id, &obj);
        fed += 1;
    }
    Ok(fed)
}

// ---------------------------------------------------------------------------
// MemIndex: the naive reference backend
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct MemIndex {
    referrers: BTreeMap<NodeId, BTreeSet<NodeId>>,
    observations: BTreeMap<StableId, BTreeSet<NodeId>>,
    entity_nodes: BTreeMap<StableId, BTreeSet<NodeId>>,
    entities_by_kind: BTreeMap<String, BTreeSet<NodeId>>,
    evidence: BTreeMap<NodeId, BTreeSet<NodeId>>,
    receipts: BTreeMap<NodeId, BTreeSet<NodeId>>,
}

impl MemIndex {
    pub fn new() -> Self {
        Self::default()
    }
}

fn sorted(set: Option<&BTreeSet<NodeId>>) -> Vec<NodeId> {
    set.map(|s| s.iter().copied().collect()).unwrap_or_default()
}

impl Index for MemIndex {
    fn on_object(&mut self, id: &NodeId, obj: &Object) {
        for (kind, target) in object_edges(obj) {
            self.referrers.entry(target).or_default().insert(*id);
            match kind {
                EdgeKind::EvidenceSubject => {
                    self.evidence.entry(target).or_default().insert(*id);
                }
                EdgeKind::ReceiptIntent => {
                    self.receipts.entry(target).or_default().insert(*id);
                }
                EdgeKind::CodeRef | EdgeKind::NamespaceEntry | EdgeKind::NamespaceParent => {}
            }
        }
        match obj {
            Object::Observation { subject, .. } => {
                self.observations.entry(subject.clone()).or_default().insert(*id);
            }
            Object::Entity { id: stable, entity_kind, .. } => {
                self.entity_nodes.entry(stable.clone()).or_default().insert(*id);
                self.entities_by_kind
                    .entry(entity_kind.clone())
                    .or_default()
                    .insert(*id);
            }
            _ => {}
        }
    }

    fn referrers(&self, target: &NodeId) -> Vec<NodeId> {
        sorted(self.referrers.get(target))
    }

    fn observations_of(&self, subject: &StableId) -> Vec<NodeId> {
        sorted(self.observations.get(subject))
    }

    fn entity_nodes(&self, id: &StableId) -> Vec<NodeId> {
        sorted(self.entity_nodes.get(id))
    }

    fn entities_by_kind(&self, kind: &str) -> Vec<NodeId> {
        sorted(self.entities_by_kind.get(kind))
    }

    fn evidence_for(&self, subject: &NodeId) -> Vec<NodeId> {
        sorted(self.evidence.get(subject))
    }

    fn receipts_for(&self, intent: &NodeId) -> Vec<NodeId> {
        sorted(self.receipts.get(intent))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use brain_core::object::{hash_object, Literal, VerificationLevel};
    use std::collections::BTreeMap as Map;

    fn lit(i: i64) -> Term {
        Term::Lit { value: Literal::Int { value: i } }
    }

    #[test]
    fn code_refs_are_extracted_from_nested_terms() {
        let target = hash_object(&Object::Code { term: lit(1) }).unwrap();
        let term = Term::Let {
            name: "x".to_string(),
            value: Box::new(Term::RefNode { node: target }),
            body: Box::new(Term::Foreign {
                symbol: "core/add".to_string(),
                arg: Box::new(Term::Record {
                    fields: {
                        let mut f = Map::new();
                        f.insert("a".to_string(), Term::Var { name: "x".to_string() });
                        f.insert("b".to_string(), Term::RefNode { node: target });
                        f
                    },
                }),
            }),
        };
        let edges = object_edges(&Object::Code { term });
        assert_eq!(edges, vec![(EdgeKind::CodeRef, target), (EdgeKind::CodeRef, target)]);
    }

    #[test]
    fn replay_builds_queryable_index_and_rebuild_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();

        // A dependency, and code that refs it.
        let dep = store.put(&Object::Code { term: lit(7) }).unwrap();
        let code = store
            .put(&Object::Code { term: Term::RefNode { node: dep } })
            .unwrap();
        store.bind("lib/seven-plus", code).unwrap();

        // A twinned entity with an observation and evidence about the code.
        let sid = StableId::derive(&["file", "src/lib.rs"]);
        let entity = store
            .put(&Object::Entity {
                id: sid.clone(),
                entity_kind: "source_file".to_string(),
                labels: Map::new(),
            })
            .unwrap();
        let obs = store
            .put(&Object::Observation {
                subject: sid.clone(),
                property: "content_b3".to_string(),
                value: "abc".to_string(),
                source: "test".to_string(),
                observed_at_ms: 1,
            })
            .unwrap();
        let ev = store
            .put(&Object::Evidence {
                subject: code,
                level: VerificationLevel::Behavioral,
                method: "unit-test".to_string(),
                passed: true,
                detail: "ok".to_string(),
            })
            .unwrap();

        // An intent and its receipt.
        let intent = store
            .put(&Object::Intent {
                action: "io/echo".to_string(),
                arg_hash: dep,
                capability: Some("io".to_string()),
                at_ms: 1,
            })
            .unwrap();
        let receipt = store
            .put(&Object::Receipt {
                intent,
                ok: true,
                detail: "done".to_string(),
                at_ms: 2,
            })
            .unwrap();

        let mut index = MemIndex::new();
        let fed = replay(&store, &mut index).unwrap();
        assert!(fed >= 6);

        // Reverse edges: the dep is referenced by the code object (and the
        // intent's arg_hash deliberately does NOT create an edge).
        let dep_referrers = index.referrers(&dep);
        assert!(dep_referrers.contains(&code));
        assert!(!dep_referrers.contains(&intent));

        // The code node is referenced by the namespace that binds it and by
        // the evidence about it.
        let code_referrers = index.referrers(&code);
        assert!(code_referrers.contains(&ev));
        assert_eq!(code_referrers.len(), 2, "evidence + binding namespace");

        assert_eq!(index.observations_of(&sid), vec![obs]);
        assert_eq!(index.entity_nodes(&sid), vec![entity]);
        assert_eq!(index.entities_by_kind("source_file"), vec![entity]);
        assert_eq!(index.evidence_for(&code), vec![ev]);
        assert_eq!(index.receipts_for(&intent), vec![receipt]);

        // Rebuild = replay again into the same index: results are unchanged.
        replay(&store, &mut index).unwrap();
        assert_eq!(index.observations_of(&sid), vec![obs]);
        assert_eq!(index.referrers(&code).len(), 2);
    }
}
