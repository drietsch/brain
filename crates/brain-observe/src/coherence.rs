//! Coherence: cross-artifact integrity findings.
//!
//! Each check compares two things the graph claims are related and flags
//! the pairs that no longer line up: an active document naming a deleted
//! file, a test case defined in a file that is gone, a governed change
//! stuck between states, a feature that says shipped while its definition
//! of done disagrees. Derived at query time, never stored — incoherence
//! is a judgment about now.

use crate::lifecycle;
use crate::twin::{latest, live_from, sid_label};
use brain_core::object::Object;
use brain_index::{Index, MemIndex};
use brain_store::{Store, StoreError};
use std::collections::BTreeSet;
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub kind: String,
    pub label: String,
    pub detail: String,
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {} — {}", self.kind, self.label, self.detail)
    }
}

/// Run every coherence check under a prefix. Deterministic order.
pub fn check(store: &Store, index: &MemIndex, prefix: &str) -> Result<Vec<Finding>, StoreError> {
    let spine = crate::spine::build(store, index, prefix)?;
    check_with(store, index, prefix, &spine)
}

/// The same, against a spine the caller already built. Eyes holds one per
/// graph version; building a second here would be pure waste.
pub fn check_with(
    store: &Store,
    index: &MemIndex,
    prefix: &str,
    spine: &crate::spine::Spine,
) -> Result<Vec<Finding>, StoreError> {
    let mut out = Vec::new();

    // Active documents holding live edges to deleted files.
    let doc_kinds = crate::kinds::doc_kinds(store, index)?;
    for kind in &doc_kinds {
        let mut seen = BTreeSet::new();
        for node in index.entities_by_kind(kind) {
            let Ok(Object::Entity { id, labels, .. }) = store.get(&node) else {
                continue;
            };
            if labels.get("prefix").map(String::as_str) != Some(prefix) || !seen.insert(id.clone())
            {
                continue;
            }
            let slug = labels.get("slug").cloned().unwrap_or_default();
            let (state, _) = lifecycle::of(index, store, &id)?;
            if !state.is_active() {
                // An explicit `active` that only survives because explicit
                // sets outrank presence is itself worth flagging.
                continue;
            }
            let mut gone = Vec::new();
            for (_, to) in live_from(index, store, &id, "mentions")? {
                if latest(index, store, &to, "present")?.as_deref() == Some("false") {
                    gone.push(sid_label(index, store, &to));
                }
            }
            if !gone.is_empty() {
                gone.sort();
                out.push(Finding {
                    kind: format!("dangling-mention ({kind})"),
                    label: slug.clone(),
                    detail: format!("names deleted file(s): {}", gone.join(", ")),
                });
            }
            // Explicitly active while every home file is deleted.
            if latest(index, store, &id, "lifecycle")?.as_deref() == Some("active") {
                let homes = live_from(index, store, &id, "recorded_in")?;
                if !homes.is_empty() {
                    let mut all_gone = true;
                    for (_, file) in &homes {
                        if latest(index, store, file, "present")?.as_deref() != Some("false") {
                            all_gone = false;
                            break;
                        }
                    }
                    if all_gone {
                        out.push(Finding {
                            kind: format!("active-but-homeless ({kind})"),
                            label: slug,
                            detail:
                                "explicitly active, but every file it is recorded in is deleted"
                                    .to_string(),
                        });
                    }
                }
            }
        }
    }

    // Test cases defined in files that no longer exist.
    let mut seen = BTreeSet::new();
    for node in index.entities_by_kind("test_case") {
        let Ok(Object::Entity { id, labels, .. }) = store.get(&node) else {
            continue;
        };
        if labels.get("prefix").map(String::as_str) != Some(prefix) || !seen.insert(id.clone()) {
            continue;
        }
        for (_, file) in live_from(index, store, &id, "defined_in")? {
            if latest(index, store, &file, "present")?.as_deref() == Some("false") {
                out.push(Finding {
                    kind: "dangling-test".to_string(),
                    label: sid_label(index, store, &id),
                    detail: format!("defined in deleted file {}", sid_label(index, store, &file)),
                });
            }
        }
    }

    // Governed changes stuck between states.
    let mut seen = BTreeSet::new();
    for node in index.entities_by_kind("change") {
        let Ok(Object::Entity { id, labels, .. }) = store.get(&node) else {
            continue;
        };
        if labels.get("prefix").map(String::as_str) != Some(prefix) || !seen.insert(id.clone()) {
            continue;
        }
        let slug = labels.get("slug").cloned().unwrap_or_default();
        match latest(index, store, &id, "status")?.as_deref() {
            Some("indeterminate") => out.push(Finding {
                kind: "stuck-change".to_string(),
                label: slug,
                detail: "indeterminate after a crash — inspect, then re-apply or revert"
                    .to_string(),
            }),
            Some("broken") => out.push(Finding {
                kind: "broken-change".to_string(),
                label: slug,
                detail: "applied but its verification run failed".to_string(),
            }),
            _ => {}
        }
    }

    // Assets whose owner has left the present.
    for (path, why) in crate::assets::orphaned(store, index, prefix)? {
        out.push(Finding {
            kind: "orphaned-asset".to_string(),
            label: path,
            detail: format!("{why} — archive or re-attach (`brain tidy`)"),
        });
    }

    // Features claiming shipped while the definition of done disagrees.
    for row in crate::features::list(store, index, prefix)? {
        if row.status == "shipped" {
            let report = crate::features::evaluate(store, index, prefix, &row.slug)?;
            if !report.done {
                let met = report.checks.iter().filter(|c| c.count > 0).count();
                out.push(Finding {
                    kind: "incoherent-feature".to_string(),
                    label: row.slug,
                    detail: format!("status 'shipped' but DoD {met}/{}", report.checks.len()),
                });
            }
        }
    }

    // Declared slots nothing observed corroborates.
    //
    // One finding, never one per feature: seventeen rows all reading
    // "claims something nothing backs up" are one thing to know about,
    // seventeen times, and ADR-029 calls that worse than not collapsing.
    //
    // What is *unclaimed* is deliberately not a finding at all. Coverage
    // is a census with its own readout on the Features surface; repeating
    // it here as eleven concerns would turn the home screen into the
    // spreadsheet this product exists to replace.
    //
    // Gated on the spine having been asked anything: on a graph where no
    // feature declares a thing, silence is the honest answer.
    if spine.asked() && !spine.uncorroborated().is_empty() {
        let rows = spine.uncorroborated();
        let features: BTreeSet<&str> = rows.iter().map(|row| row.slug.as_str()).collect();
        let examples: Vec<String> = rows
            .iter()
            .take(3)
            .map(|row| {
                let what = row
                    .targets
                    .first()
                    .map(|sid| sid_label(index, store, sid))
                    .unwrap_or_default();
                format!("{} claims {what}", row.slug)
            })
            .collect();
        let rest = rows.len().saturating_sub(examples.len());
        out.push(Finding {
            kind: "uncorroborated-claim".to_string(),
            label: format!(
                "{} feature{}",
                features.len(),
                if features.len() == 1 { "" } else { "s" }
            ),
            detail: if rest > 0 {
                format!("{}, and {rest} more", examples.join("; "))
            } else {
                examples.join("; ")
            },
        });
    }

    out.sort_by(|a, b| a.kind.cmp(&b.kind).then(a.label.cmp(&b.label)));
    Ok(out)
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
    fn findings_flag_dangling_mentions_tests_changes_and_features() {
        let src = tempfile::tempdir().unwrap();
        fs::create_dir_all(src.path().join("src")).unwrap();
        fs::create_dir_all(src.path().join("docs/adr")).unwrap();
        fs::write(src.path().join("src/core.rs"), "pub fn c() {}\n").unwrap();
        fs::write(
            src.path().join("docs/adr/adr-001-core.md"),
            "# Core\n\nStatus: accepted\n\nAll about src/core.rs.\n",
        )
        .unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let store = Store::open(store_dir.path()).unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();

        // A test case defined in a file, then the file (and mention target)
        // are deleted.
        let junit =
            "<testsuite>\n  <testcase classname=\"src/core.rs\" name=\"works\"/>\n</testsuite>\n";
        crate::testing::record_run(
            &store,
            "twin/app",
            &crate::testing::parse_report(junit),
            junit,
        )
        .unwrap();
        fs::remove_file(src.path().join("src/core.rs")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(3));
        refresh(&store, src.path(), "twin/app").unwrap();

        // A feature that claims shipped with nothing linked.
        crate::features::add(&store, "twin/app", "checkout", "Checkout", "shipped").unwrap();

        let index = fresh_index(&store);
        let findings = check(&store, &index, "twin/app").unwrap();
        let kinds: Vec<&str> = findings.iter().map(|f| f.kind.as_str()).collect();
        assert!(
            kinds.contains(&"dangling-mention (decision)"),
            "doc names a deleted file: {findings:?}"
        );
        assert!(kinds.contains(&"dangling-test"), "{findings:?}");
        assert!(kinds.contains(&"incoherent-feature"), "{findings:?}");

        // Deterministic and read-only.
        let before = store.count_objects().unwrap();
        assert_eq!(findings, check(&store, &index, "twin/app").unwrap());
        assert_eq!(store.count_objects().unwrap(), before);
    }
}
