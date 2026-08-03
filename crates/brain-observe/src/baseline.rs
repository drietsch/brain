//! Named baselines: moments worth returning to, recorded on the repo
//! entity as observations.
//!
//! The value carries the moment it names (`name@ms`) while the
//! observation keeps its own honest timestamp — recorded now, naming
//! then. A backdated observation would splice itself into a past
//! refresh's batch wherever episodes and co-change group by timestamp,
//! and quietly lie in the event log. Baselines are append-only; there
//! is no retraction, because a moment that stopped mattering simply
//! stops being asked about.

use brain_core::ids::StableId;
use brain_core::object::Object;
use brain_index::{Index, MemIndex};
use brain_store::{now_ms, Store, StoreError};

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Baseline {
    pub name: String,
    /// The moment the baseline names.
    pub at_ms: u64,
    /// When someone recorded it.
    pub recorded_at_ms: u64,
}

/// Record a baseline naming `at_ms`. A duplicate name is refused with
/// advice rather than silently shadowed — a baseline that can move is
/// not a baseline.
pub fn add(
    store: &Store,
    index: &MemIndex,
    prefix: &str,
    name: &str,
    at_ms: u64,
) -> Result<(), String> {
    let ok = !name.is_empty()
        && !name.contains('@')
        && !name.chars().any(|c| c.is_whitespace());
    if !ok {
        return Err(format!(
            "'{name}' cannot name a baseline — pick a single word without '@'"
        ));
    }
    let taken = list(store, index, prefix)
        .map_err(|e| e.to_string())?
        .iter()
        .any(|b| b.name == name);
    if taken {
        return Err(format!(
            "baseline '{name}' already names a moment — pick another name"
        ));
    }
    let repo = StableId::derive(&["repo", prefix]);
    crate::twin::observe_src(
        store,
        &repo,
        "baseline",
        &format!("{name}@{at_ms}"),
        "agent",
        now_ms(),
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Every baseline under the prefix, the moment it names first —
/// newest-named leading.
pub fn list(store: &Store, index: &MemIndex, prefix: &str) -> Result<Vec<Baseline>, StoreError> {
    let repo = StableId::derive(&["repo", prefix]);
    let mut out = Vec::new();
    for id in index.observations_of(&repo) {
        if let Object::Observation {
            property,
            value,
            observed_at_ms,
            ..
        } = store.get(&id)?
        {
            if property != "baseline" {
                continue;
            }
            // The name may not contain '@', so the last '@' splits it
            // from the moment no matter what the name looks like.
            if let Some((name, at)) = value.rsplit_once('@') {
                if let Ok(at_ms) = at.parse() {
                    out.push(Baseline {
                        name: name.to_string(),
                        at_ms,
                        recorded_at_ms: observed_at_ms,
                    });
                }
            }
        }
    }
    out.sort_by(|a, b| b.at_ms.cmp(&a.at_ms).then(a.name.cmp(&b.name)));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use brain_index::replay;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        (dir, store)
    }

    fn index_of(store: &Store) -> MemIndex {
        let mut index = MemIndex::new();
        replay(store, &mut index).unwrap();
        index
    }

    #[test]
    fn a_baseline_names_a_moment_once() {
        let (_dir, store) = store();
        let index = index_of(&store);
        add(&store, &index, "twin/app", "v1-launch", 1000).unwrap();

        let index = index_of(&store);
        let listed = list(&store, &index, "twin/app").unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "v1-launch");
        assert_eq!(listed[0].at_ms, 1000);
        assert!(listed[0].recorded_at_ms >= 1000);

        // The same name cannot quietly move to a new moment.
        let err = add(&store, &index, "twin/app", "v1-launch", 2000).unwrap_err();
        assert!(err.contains("already names a moment"), "{err}");

        // Names that would break the encoding or the reader are refused.
        for bad in ["", "two words", "with@sign"] {
            assert!(add(&store, &index, "twin/app", bad, 3000).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn baselines_list_newest_named_first() {
        let (_dir, store) = store();
        let index = index_of(&store);
        add(&store, &index, "twin/app", "older", 1000).unwrap();
        let index = index_of(&store);
        add(&store, &index, "twin/app", "newer", 2000).unwrap();
        let index = index_of(&store);
        let listed = list(&store, &index, "twin/app").unwrap();
        assert_eq!(
            listed.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(),
            vec!["newer", "older"]
        );
    }
}
