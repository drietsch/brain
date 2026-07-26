//! Replication: content-addressed sync between stores.
//!
//! This is how code (and everything else in the graph) moves — it replaces
//! deployment in the no-files world. The design falls out of the invariants:
//!
//! - **Objects are a conflict-free set union.** Immutable + content-addressed
//!   means two stores can never disagree about an object, only about whether
//!   they have it. Every ingested object is re-hashed; a mismatch aborts.
//! - **Namespaces preserve conflicts as explicit structure.** If both stores
//!   bound the same name to different nodes, the destination's binding is
//!   kept and the source's target is bound under `sync-conflict/<name>` —
//!   disagreement stays visible in the graph for deliberate resolution,
//!   never silently overwritten.
//! - **Operational state stays local.** The intent log (pending/indeterminate
//!   state machine) does not travel; receipts and evidence do, because they
//!   are graph objects. A program verified in one store arrives in another
//!   with its evidence attached.
//!
//! `pull(dest, source)` is idempotent: a second pull copies nothing and adds
//! no bindings.

use crate::{Store, StoreError};
use brain_core::ids::NodeId;
use serde_json::json;

#[derive(Debug, Default, PartialEq)]
pub struct SyncReport {
    pub objects_copied: usize,
    pub objects_present: usize,
    pub names_added: usize,
    pub names_agreed: usize,
    /// (name, destination target kept, source target under sync-conflict/).
    pub conflicts: Vec<(String, NodeId, NodeId)>,
}

/// Pull everything the source has into the destination.
pub fn pull(dest: &Store, source: &Store) -> Result<SyncReport, StoreError> {
    let mut report = SyncReport::default();

    for id in source.put_history()? {
        if dest.has(&id) {
            report.objects_present += 1;
            continue;
        }
        let obj = source.get(&id)?; // get() verifies integrity at the source
        let copied = dest.put(&obj)?;
        if copied != id {
            // Never silently accept an object under a different identity
            // than it claimed. This fires when the source store was written
            // under an older canonicalization (e.g. pre-alpha-normalization).
            return Err(StoreError::CanonEpoch { claimed: id, actual: copied });
        }
        report.objects_copied += 1;
    }

    let src_ns = source.namespace()?;
    let dst_ns = dest.namespace()?;
    let mut adds: Vec<(String, NodeId)> = Vec::new();
    for (name, target) in src_ns {
        match dst_ns.get(&name) {
            None => {
                adds.push((name, target));
                report.names_added += 1;
            }
            Some(t) if *t == target => report.names_agreed += 1,
            Some(t) => {
                let alias = format!("sync-conflict/{name}");
                if dst_ns.get(&alias) != Some(&target) {
                    adds.push((alias, target));
                }
                report.conflicts.push((name, *t, target));
            }
        }
    }
    if !adds.is_empty() {
        dest.bind_many(adds)?;
    }

    dest.append_event(
        "sync",
        json!({
            "source": source.root().display().to_string(),
            "objects_copied": report.objects_copied,
            "names_added": report.names_added,
            "conflicts": report.conflicts.len(),
        }),
    )?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use brain_core::object::{Literal, Object, Term, VerificationLevel};

    fn code(i: i64) -> Object {
        Object::Code {
            term: Term::Lit { value: Literal::Int { value: i } },
        }
    }

    #[test]
    fn pull_copies_objects_bindings_and_evidence_idempotently() {
        let a_dir = tempfile::tempdir().unwrap();
        let b_dir = tempfile::tempdir().unwrap();
        let a = Store::open(a_dir.path()).unwrap();
        let b = Store::open(b_dir.path()).unwrap();

        let prog = a.put(&code(42)).unwrap();
        let ev = a
            .put(&Object::Evidence {
                subject: prog,
                level: VerificationLevel::Behavioral,
                method: "task:demo@abc".to_string(),
                passed: true,
                detail: "ok".to_string(),
            })
            .unwrap();
        a.bind("lib/answer", prog).unwrap();

        let r1 = pull(&b, &a).unwrap();
        assert!(r1.objects_copied >= 3, "code + evidence + namespace"); // namespaces are objects too
        assert!(r1.conflicts.is_empty());
        assert_eq!(b.resolve("lib/answer").unwrap(), Some(prog));
        assert!(b.has(&ev), "evidence travels with the program");

        // Idempotent: nothing new moves on a second pull.
        let r2 = pull(&b, &a).unwrap();
        assert_eq!(r2.objects_copied, 0);
        assert_eq!(r2.names_added, 0);
        assert!(r2.conflicts.is_empty());
    }

    #[test]
    fn conflicting_bindings_are_preserved_not_overwritten() {
        let a_dir = tempfile::tempdir().unwrap();
        let b_dir = tempfile::tempdir().unwrap();
        let a = Store::open(a_dir.path()).unwrap();
        let b = Store::open(b_dir.path()).unwrap();

        let ours = b.put(&code(1)).unwrap();
        let theirs = a.put(&code(2)).unwrap();
        b.bind("app/main", ours).unwrap();
        a.bind("app/main", theirs).unwrap();

        let report = pull(&b, &a).unwrap();
        assert_eq!(report.conflicts, vec![("app/main".to_string(), ours, theirs)]);
        // Destination keeps its binding; the source's target stays reachable.
        assert_eq!(b.resolve("app/main").unwrap(), Some(ours));
        assert_eq!(b.resolve("sync-conflict/app/main").unwrap(), Some(theirs));

        // Re-pull: the conflict is still reported but no new bindings churn.
        let again = pull(&b, &a).unwrap();
        assert_eq!(again.conflicts.len(), 1);
        assert_eq!(again.names_added, 0);
    }
}
