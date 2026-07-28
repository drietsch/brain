//! Lifecycle: is this artifact still *current*, or history?
//!
//! Derived at query time from facts the graph already holds — a live
//! incoming `supersedes` edge, an explicit `lifecycle` observation, a
//! mapped `status`, or the deletion of every file the artifact is
//! recorded in. Only explicit sets are stored (source `agent`); the
//! judgment itself is never materialized (ADR-009, ADR-013). Every list,
//! staleness check, and attention pass consumes this, so a finished plan
//! or superseded decision stops rotting the moment it leaves the present.

use crate::twin::{latest, live_from, live_to, observe_src, sid_label};
use brain_core::ids::StableId;
use brain_index::MemIndex;
use brain_store::{now_ms, Store, StoreError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifecycle {
    Active,
    Done,
    Abandoned,
    Retired,
    Superseded,
}

impl Lifecycle {
    pub fn as_str(&self) -> &'static str {
        match self {
            Lifecycle::Active => "active",
            Lifecycle::Done => "done",
            Lifecycle::Abandoned => "abandoned",
            Lifecycle::Retired => "retired",
            Lifecycle::Superseded => "superseded",
        }
    }

    pub fn parse(s: &str) -> Option<Lifecycle> {
        match s {
            "active" => Some(Lifecycle::Active),
            "done" => Some(Lifecycle::Done),
            "abandoned" => Some(Lifecycle::Abandoned),
            "retired" => Some(Lifecycle::Retired),
            "superseded" => Some(Lifecycle::Superseded),
            _ => None,
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(self, Lifecycle::Active)
    }
}

/// Map a document `status` value to a lifecycle state, when it implies one.
fn from_status(status: &str) -> Option<Lifecycle> {
    match status {
        "superseded" | "replaced" => Some(Lifecycle::Superseded),
        "deprecated" | "rejected" | "withdrawn" | "retired" => Some(Lifecycle::Retired),
        "done" | "completed" | "shipped" | "executed" | "implemented" => Some(Lifecycle::Done),
        "abandoned" => Some(Lifecycle::Abandoned),
        _ => None,
    }
}

/// The lifecycle judgment for an artifact entity, with its reason.
/// Precedence, first match wins:
/// 1. a live incoming `supersedes` edge — a structural declaration in a
///    document outranks any CLI override (undo it by editing the file);
/// 2. the latest explicit `lifecycle` observation;
/// 3. the latest `status` observation, when it implies a state;
/// 4. every `recorded_in` file deleted — the artifact's home is gone;
/// 5. active.
pub fn of(
    index: &MemIndex,
    store: &Store,
    sid: &StableId,
) -> Result<(Lifecycle, String), StoreError> {
    let by = live_to(index, store, sid, "supersedes")?;
    if let Some((_, successor)) = by.first() {
        return Ok((
            Lifecycle::Superseded,
            format!("superseded by {}", sid_label(index, store, successor)),
        ));
    }
    if let Some(v) = latest(index, store, sid, "lifecycle")? {
        if let Some(state) = Lifecycle::parse(&v) {
            let why = latest(index, store, sid, "lifecycle_why")?
                .map(|w| format!(": {w}"))
                .unwrap_or_default();
            return Ok((state, format!("set by agent{why}")));
        }
    }
    if let Some(status) = latest(index, store, sid, "status")? {
        if let Some(state) = from_status(&status) {
            return Ok((state, format!("status '{status}'")));
        }
    }
    let homes = live_from(index, store, sid, "recorded_in")?;
    if !homes.is_empty() {
        let mut all_gone = true;
        for (_, file) in &homes {
            if latest(index, store, file, "present")?.as_deref() != Some("false") {
                all_gone = false;
                break;
            }
        }
        if all_gone {
            return Ok((Lifecycle::Retired, "source file deleted".to_string()));
        }
    }
    Ok((Lifecycle::Active, String::new()))
}

/// Explicitly set an artifact's lifecycle. Guarded: setting the state it
/// already has writes nothing. Returns whether anything was written.
pub fn set(
    store: &Store,
    index: &MemIndex,
    sid: &StableId,
    state: Lifecycle,
    why: Option<&str>,
) -> Result<bool, StoreError> {
    let now = now_ms();
    let mut wrote = false;
    if latest(index, store, sid, "lifecycle")?.as_deref() != Some(state.as_str()) {
        observe_src(store, sid, "lifecycle", state.as_str(), "agent", now)?;
        wrote = true;
    }
    if let Some(w) = why {
        if latest(index, store, sid, "lifecycle_why")?.as_deref() != Some(w) {
            observe_src(store, sid, "lifecycle_why", w, "agent", now)?;
            wrote = true;
        }
    }
    Ok(wrote)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::twin::refresh;
    use brain_index::replay;
    use std::fs;

    fn fresh_index(store: &Store) -> MemIndex {
        let mut index = MemIndex::new();
        replay(store, &mut index).unwrap();
        index
    }

    #[test]
    fn derivation_precedence_supersedes_then_explicit_then_status_then_presence() {
        let src = tempfile::tempdir().unwrap();
        fs::create_dir_all(src.path().join("docs/adr")).unwrap();
        fs::create_dir_all(src.path().join("docs/plans")).unwrap();
        fs::write(
            src.path().join("docs/adr/adr-001-old.md"),
            "# Old way\n\nStatus: accepted\n",
        )
        .unwrap();
        fs::write(
            src.path().join("docs/adr/adr-002-new.md"),
            "# New way\n\nStatus: accepted\nSupersedes: adr-001-old.md\n",
        )
        .unwrap();
        fs::write(src.path().join("docs/plans/quiet.md"), "# Quiet plan\n").unwrap();
        fs::write(
            src.path().join("docs/plans/shipped.md"),
            "# Shipped\n\nStatus: done\n",
        )
        .unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let store = Store::open(store_dir.path()).unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();

        let index = fresh_index(&store);
        let old = StableId::derive(&["decision", "twin/app", "adr-001-old"]);
        let new = StableId::derive(&["decision", "twin/app", "adr-002-new"]);
        let quiet = StableId::derive(&["plan", "twin/app", "quiet"]);
        let shipped = StableId::derive(&["plan", "twin/app", "shipped"]);

        // 1. The supersedes edge retires the old decision.
        let (state, why) = of(&index, &store, &old).unwrap();
        assert_eq!(state, Lifecycle::Superseded);
        assert!(why.contains("adr-002-new"), "{why}");
        assert_eq!(of(&index, &store, &new).unwrap().0, Lifecycle::Active);

        // 3. A plan's declared status maps to Done; no status stays Active.
        assert_eq!(of(&index, &store, &shipped).unwrap().0, Lifecycle::Done);
        assert_eq!(of(&index, &store, &quiet).unwrap().0, Lifecycle::Active);

        // 2. An explicit set outranks status; setting is guarded.
        assert!(set(
            &store,
            &index,
            &shipped,
            Lifecycle::Abandoned,
            Some("rescoped")
        )
        .unwrap());
        let index = fresh_index(&store);
        let (state, why) = of(&index, &store, &shipped).unwrap();
        assert_eq!(state, Lifecycle::Abandoned);
        assert!(why.contains("rescoped"));
        let before = store.count_objects().unwrap();
        assert!(!set(
            &store,
            &index,
            &shipped,
            Lifecycle::Abandoned,
            Some("rescoped")
        )
        .unwrap());
        assert_eq!(
            store.count_objects().unwrap(),
            before,
            "guarded re-set writes nothing"
        );

        // 4. Deleting the file a doc is recorded in retires it.
        fs::remove_file(src.path().join("docs/plans/quiet.md")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(3));
        refresh(&store, src.path(), "twin/app").unwrap();
        let index = fresh_index(&store);
        let (state, why) = of(&index, &store, &quiet).unwrap();
        assert_eq!(state, Lifecycle::Retired);
        assert_eq!(why, "source file deleted");

        // 1 beats 2: an explicit `active` cannot resurrect a superseded doc.
        set(&store, &index, &old, Lifecycle::Active, None).unwrap();
        let index = fresh_index(&store);
        assert_eq!(of(&index, &store, &old).unwrap().0, Lifecycle::Superseded);
    }
}
