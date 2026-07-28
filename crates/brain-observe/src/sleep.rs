//! Consolidation: the rest-cycle — distill activity into durable memory.
//!
//! `sleep` reads everything observed since the last consolidation and
//! writes back *summaries*: a per-file `memory` digest for files with
//! real history, and a repo-level `session_summary` — the orientation
//! narrative the next session reads instead of replaying raw history.
//! Immutability is preserved: consolidation adds memory, it never removes
//! detail. All writes are guarded and sourced `"sleep"`; a second sleep
//! with no new activity writes nothing.

use crate::twin::{latest, latest_at, observe_src};
use brain_core::ids::StableId;
use brain_core::object::Object;
use brain_index::{replay, Index, MemIndex};
use brain_store::{now_ms, Store, StoreError};

#[derive(Debug)]
pub struct SleepReport {
    pub summary: String,
    /// Per-file memory digests written or refreshed this sleep.
    pub memories: usize,
    pub wrote: bool,
}

/// What happened under a prefix since `since` — the shared delta both
/// consolidation (`sleep`) and orientation (`wake`) are built from.
#[derive(Debug, Default)]
pub struct Delta {
    pub added: Vec<String>,
    pub changed: Vec<String>,
    pub notes: usize,
    pub doc_updates: usize,
    pub new_runs: usize,
    /// Rendered latest-run verdict ("; last run 88/88 ok"), or empty.
    pub verdict: String,
    /// Every present file with its content-version count (memory input).
    pub file_versions: Vec<(StableId, u32)>,
}

impl Delta {
    pub fn activity(&self) -> usize {
        self.added.len() + self.changed.len() + self.doc_updates + self.new_runs + self.notes
    }
}

/// Compute the delta since a watermark (normally `consolidated_until`).
pub fn delta_since(
    store: &Store,
    index: &MemIndex,
    prefix: &str,
    since: u64,
) -> Result<Delta, StoreError> {
    let mut delta = Delta::default();
    let repo_sid = StableId::derive(&["repo", prefix]);

    let ns = store.namespace()?;
    for (name, node) in &ns {
        let Some(rel) = name.strip_prefix(&format!("{prefix}/")) else {
            continue;
        };
        let Ok(Object::Entity {
            id: sid,
            entity_kind,
            ..
        }) = store.get(node)
        else {
            continue;
        };
        if entity_kind != "source_file" {
            continue;
        }
        let mut versions = 0u32;
        let mut first_at = u64::MAX;
        let mut newest_change = 0u64;
        for oid in index.observations_of(&sid) {
            if let Object::Observation {
                property,
                observed_at_ms,
                ..
            } = store.get(&oid)?
            {
                match property.as_str() {
                    "content_b3" => {
                        versions += 1;
                        first_at = first_at.min(observed_at_ms);
                        newest_change = newest_change.max(observed_at_ms);
                    }
                    "note" if observed_at_ms > since => delta.notes += 1,
                    _ => {}
                }
            }
        }
        if newest_change > since {
            if first_at > since {
                delta.added.push(rel.to_string());
            } else {
                delta.changed.push(rel.to_string());
            }
        }
        delta.file_versions.push((sid, versions));
    }
    // Repo-level notes count toward the delta too.
    for oid in index.observations_of(&repo_sid) {
        if let Object::Observation {
            property,
            observed_at_ms,
            ..
        } = store.get(&oid)?
        {
            if property == "note" && observed_at_ms > since {
                delta.notes += 1;
            }
        }
    }

    // Documents (built-in and graph-taught kinds) updated since.
    let doc_kinds = crate::kinds::doc_kinds(store, index)?;
    for kind in &doc_kinds {
        let mut seen = std::collections::BTreeSet::new();
        for node in index.entities_by_kind(kind) {
            let Ok(Object::Entity { id, labels, .. }) = store.get(&node) else {
                continue;
            };
            if labels.get("prefix").map(String::as_str) != Some(prefix) || !seen.insert(id.clone())
            {
                continue;
            }
            if latest_at(index, store, &id, "content")?.is_some_and(|(at, _)| at > since) {
                delta.doc_updates += 1;
            }
        }
    }

    // Test protocols imported since, and the latest verdict.
    let runs = crate::testing::runs(store, index, prefix)?;
    delta.new_runs = runs.iter().filter(|(at, ..)| *at > since).count();
    delta.verdict = runs
        .first()
        .map(|(_, total, passed, failed, _)| {
            if *failed == 0 {
                format!("; last run {passed}/{total} ok")
            } else {
                format!("; last run {failed} FAILING")
            }
        })
        .unwrap_or_default();
    Ok(delta)
}

pub fn sleep(store: &Store, prefix: &str) -> Result<SleepReport, StoreError> {
    let mut index = MemIndex::new();
    replay(store, &mut index)?;
    let now = now_ms();
    let repo_sid = StableId::derive(&["repo", prefix]);
    let since: u64 = latest(&index, store, &repo_sid, "consolidated_until")?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    let delta = delta_since(store, &index, prefix, since)?;

    // Memory digest for files with real history (≥3 versions).
    let mut memories = 0usize;
    for (sid, versions) in &delta.file_versions {
        if *versions >= 3 {
            let symbols = crate::twin::live_from(&index, store, sid, "contains")?.len();
            let declared: u32 = latest(&index, store, sid, "tests_declared")?
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            let tests = if declared > 0 {
                format!("; tests {declared}")
            } else {
                String::new()
            };
            let digest = format!("v{versions}; {symbols} symbol(s){tests}");
            if latest(&index, store, sid, "memory")?.as_deref() != Some(digest.as_str()) {
                observe_src(store, sid, "memory", &digest, "sleep", now)?;
                memories += 1;
            }
        }
    }

    let (added, changed) = (delta.added.len(), delta.changed.len());
    let Delta {
        notes,
        doc_updates,
        new_runs,
        verdict,
        ..
    } = delta;
    let activity = added + changed + doc_updates + new_runs + notes + memories;
    if activity == 0 {
        return Ok(SleepReport {
            summary: "nothing new since last sleep".to_string(),
            memories: 0,
            wrote: false,
        });
    }

    // The orientation narrative, ending with where attention points now.
    let top: Vec<String> = crate::attention::attend(store, &index, prefix)?
        .into_iter()
        .take(3)
        .map(|a| a.label)
        .collect();
    let attention = if top.is_empty() {
        String::new()
    } else {
        format!("; attention: {}", top.join(", "))
    };
    let summary = format!(
        "{added} added, {changed} changed file(s); {doc_updates} doc update(s); \
         {new_runs} protocol(s){verdict}; {notes} note(s); {memories} memory digest(s){attention}"
    );
    observe_src(store, &repo_sid, "session_summary", &summary, "sleep", now)?;
    observe_src(
        store,
        &repo_sid,
        "consolidated_until",
        &now.to_string(),
        "sleep",
        now,
    )?;
    Ok(SleepReport {
        summary,
        memories,
        wrote: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::twin::refresh;
    use std::fs;

    #[test]
    fn sleep_consolidates_activity_then_rests_idempotently() {
        let src = tempfile::tempdir().unwrap();
        fs::create_dir_all(src.path().join("src")).unwrap();
        fs::write(src.path().join("src/main.rs"), "pub fn main() {}\n").unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let store = Store::open(store_dir.path()).unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        // Two edits -> three content versions: enough history for a memory.
        for i in 0..2 {
            fs::write(
                src.path().join("src/main.rs"),
                format!("pub fn main() {{ /* {i} */ }}\n"),
            )
            .unwrap();
            refresh(&store, src.path(), "twin/app").unwrap();
        }
        crate::twin::add_note(
            &store,
            &StableId::derive(&["file", "src/main.rs"]),
            "iterated twice",
        )
        .unwrap();

        let report = sleep(&store, "twin/app").unwrap();
        assert!(report.wrote);
        assert!(report.summary.contains("1 added"), "{}", report.summary);
        assert!(report.summary.contains("1 note(s)"), "{}", report.summary);
        assert_eq!(report.memories, 1, "main.rs earned a memory digest");
        assert!(report.summary.contains("attention:"), "{}", report.summary);

        // The memory and the narrative are durable graph facts.
        let mut index = MemIndex::new();
        replay(&store, &mut index).unwrap();
        let main_sid = StableId::derive(&["file", "src/main.rs"]);
        let memory = latest(&index, &store, &main_sid, "memory")
            .unwrap()
            .unwrap();
        assert!(memory.starts_with("v3;"), "{memory}");
        let repo = StableId::derive(&["repo", "twin/app"]);
        assert!(latest(&index, &store, &repo, "session_summary")
            .unwrap()
            .is_some());

        // A second sleep with no new activity writes nothing at all.
        std::thread::sleep(std::time::Duration::from_millis(5));
        let before = store.count_objects().unwrap();
        let again = sleep(&store, "twin/app").unwrap();
        assert!(!again.wrote);
        assert_eq!(again.summary, "nothing new since last sleep");
        assert_eq!(store.count_objects().unwrap(), before, "rest is restful");

        // New activity after sleeping consolidates as a fresh delta.
        std::thread::sleep(std::time::Duration::from_millis(5));
        fs::write(src.path().join("src/util.rs"), "pub fn helper() {}\n").unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        let report = sleep(&store, "twin/app").unwrap();
        assert!(report.wrote);
        assert!(
            report.summary.contains("1 added, 0 changed"),
            "{}",
            report.summary
        );
    }
}
