//! Attention: the salience engine — *what deserves attention now*.
//!
//! Computed at query time from signals the graph already holds (churn,
//! blast radius, missing tests, failing protocols, stale or nonconforming
//! docs, incoherent features) and never stored: salience is a judgment
//! about the present, not a fact. Integer weights keep ranking
//! deterministic.

use crate::twin::{self, latest};
use brain_core::ids::StableId;
use brain_core::object::Object;
use brain_index::{Index, MemIndex};
use brain_store::{Store, StoreError};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub struct Attention {
    pub label: String,
    pub kind: String,
    pub score: u32,
    pub reasons: Vec<String>,
}

/// Rank everything under a prefix by salience, highest first.
///
/// Churn is recency-weighted: edits since the last sleep dominate, while
/// lifetime edit counts are capped — a file that was busy a year ago must
/// not outrank the file being worked on today. The window needs no clock
/// parameter: `consolidated_until` (written by `brain sleep`) is the
/// natural boundary of "now", and a store that never slept scores every
/// edit as recent, which matches its reality.
pub fn attend(store: &Store, index: &MemIndex, prefix: &str) -> Result<Vec<Attention>, StoreError> {
    let ins = twin::insights_with(store, index, prefix)?;
    attend_with(store, index, prefix, &ins)
}

/// Rank attention using an already-computed insights projection. Human
/// interfaces often need both summaries at once; sharing the expensive
/// synthesis keeps the read side responsive without caching graph truth.
pub fn attend_with(
    store: &Store,
    index: &MemIndex,
    prefix: &str,
    ins: &twin::Insights,
) -> Result<Vec<Attention>, StoreError> {
    let mut out: Vec<Attention> = Vec::new();

    let repo_sid = StableId::derive(&["repo", prefix]);
    let since: u64 = latest(index, store, &repo_sid, "consolidated_until")?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    // Failing test cases, attributed to files where defined_in links exist.
    let mut failing_by_file: BTreeMap<StableId, u32> = BTreeMap::new();
    for node in index.entities_by_kind("test_case") {
        let Ok(Object::Entity { id, labels, .. }) = store.get(&node) else {
            continue;
        };
        if labels.get("prefix").map(String::as_str) != Some(prefix) {
            continue;
        }
        if latest(index, store, &id, "result")?.as_deref() != Some("fail") {
            continue;
        }
        for (_, to) in twin::live_from(index, store, &id, "defined_in")? {
            *failing_by_file.entry(to).or_default() += 1;
        }
    }

    // File signals: full pass, untruncated (insights caps its lists).
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
        if entity_kind != "source_file"
            || latest(index, store, &sid, "present")?.as_deref() == Some("false")
            // Generated projections regenerate constantly; never salient.
            || latest(index, store, &sid, "generated")?.as_deref() == Some("true")
        {
            continue;
        }
        let mut score = 0u32;
        let mut reasons = Vec::new();
        let (mut lifetime, mut recent) = (0u32, 0u32);
        for id in index.observations_of(&sid) {
            if let Ok(Object::Observation {
                property,
                observed_at_ms,
                ..
            }) = store.get(&id)
            {
                if property == "content_b3" {
                    lifetime += 1;
                    if observed_at_ms > since {
                        recent += 1;
                    }
                }
            }
        }
        if recent > 0 || lifetime > 1 {
            score += recent * 4 + lifetime.min(10);
            reasons.push(format!("churn {lifetime} ({recent} recent)"));
        }
        let importers = twin::live_to(index, store, &sid, "imports")?.len() as u32;
        if importers > 0 {
            score += importers * 2;
            reasons.push(format!("hub {importers}"));
        }
        let declared: u32 = latest(index, store, &sid, "tests_declared")?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        if importers > 0 && declared == 0 && twin::live_to(index, store, &sid, "covers")?.is_empty()
        {
            score += importers * 3;
            reasons.push("untested hub".to_string());
        }
        if let Some(f) = failing_by_file.get(&sid) {
            score += f * 5;
            reasons.push(format!("{f} failing test(s)"));
        }
        if score > 0 {
            out.push(Attention {
                label: rel.to_string(),
                kind: "file".to_string(),
                score,
                reasons,
            });
        }
    }

    // Doc signals: stale beats nonconforming; a doc can be both. Severity
    // follows the kind's rot policy — records murmur, living docs shout.
    let mut docs: BTreeMap<(String, String), (u32, Vec<String>)> = BTreeMap::new();
    for stale in &ins.stale_docs {
        let e = docs
            .entry((stale.slug.clone(), stale.kind.clone()))
            .or_default();
        match stale.severity {
            twin::Severity::Warn => {
                e.0 += 4 + stale.changed.len() as u32;
                e.1.push(format!("stale: {} changed since", stale.changed.join(", ")));
            }
            twin::Severity::Info => {
                e.0 += 1;
                e.1.push(format!(
                    "stale (info): {} changed since",
                    stale.changed.join(", ")
                ));
            }
        }
    }
    for (slug, kind, missing) in &ins.nonconforming {
        let e = docs.entry((slug.clone(), kind.clone())).or_default();
        e.0 += 2;
        e.1.push(format!("nonconforming: missing {missing}"));
    }
    for ((label, kind), (score, reasons)) in docs {
        out.push(Attention {
            label,
            kind,
            score,
            reasons,
        });
    }

    // Feature incoherence: claims to be shipped, graph says not done.
    for (slug, status, fraction) in &ins.features {
        let done = fraction.split_once('/').is_some_and(|(a, b)| a == b);
        if status == "shipped" && !done {
            out.push(Attention {
                label: slug.clone(),
                kind: "feature".to_string(),
                score: 4,
                reasons: vec![format!("status '{status}' but DoD {fraction}")],
            });
        }
    }

    out.sort_by(|a, b| b.score.cmp(&a.score).then(a.label.cmp(&b.label)));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::twin::refresh;
    use brain_index::replay;
    use std::fs;

    #[test]
    fn salience_ranks_churned_untested_hubs_and_stale_docs() {
        let src = tempfile::tempdir().unwrap();
        fs::create_dir_all(src.path().join("web")).unwrap();
        fs::create_dir_all(src.path().join("docs/adr")).unwrap();
        // util.js is a hub (imported), untested, and will churn.
        fs::write(src.path().join("web/util.js"), "export function h() {}\n").unwrap();
        fs::write(src.path().join("web/a.js"), "import { h } from './util';\n").unwrap();
        fs::write(src.path().join("web/b.js"), "import { h } from './util';\n").unwrap();
        fs::write(
            src.path().join("docs/adr/adr-001-x.md"),
            "# X\n\nStatus: accepted\n\nAbout web/a.js.\n",
        )
        .unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let store = brain_store::Store::open(store_dir.path()).unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        for i in 0..2 {
            fs::write(
                src.path().join("web/util.js"),
                format!("export function h() {{ return {i} }}\n"),
            )
            .unwrap();
            refresh(&store, src.path(), "twin/app").unwrap();
        }
        // The mentioned file changes after the ADR: the doc goes stale.
        std::thread::sleep(std::time::Duration::from_millis(5));
        fs::write(
            src.path().join("web/a.js"),
            "import { h } from './util';\nh();\n",
        )
        .unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();

        let mut index = MemIndex::new();
        replay(&store, &mut index).unwrap();
        let ranked = attend(&store, &index, "twin/app").unwrap();
        assert!(!ranked.is_empty());
        // Never slept: every edit is recent. util.js: churn 3 recent
        // (3×4 + 3 = 15) + hub 2 (×2=4) + untested hub (2×3=6) = 25.
        assert_eq!(ranked[0].label, "web/util.js", "ranked: {ranked:?}");
        assert_eq!(ranked[0].score, 25);
        assert!(ranked[0].reasons.iter().any(|r| r == "churn 3 (3 recent)"));
        assert!(ranked[0].reasons.iter().any(|r| r == "untested hub"));
        // The stale ADR appears with its reason.
        let doc = ranked
            .iter()
            .find(|a| a.kind == "decision")
            .expect("stale doc ranked");
        assert_eq!(doc.label, "adr-001-x");
        assert!(doc.reasons[0].contains("stale"));
        // Determinism: recompute gives the identical ranking.
        let again = attend(&store, &index, "twin/app").unwrap();
        assert_eq!(ranked, again);

        // A generated projection, however churned, is never salient.
        let util = StableId::derive(&["file", "web/util.js"]);
        crate::twin::observe_src(&store, &util, "generated", "true", "docsgen", 999).unwrap();
        let mut index = MemIndex::new();
        replay(&store, &mut index).unwrap();
        let ranked = attend(&store, &index, "twin/app").unwrap();
        assert!(
            !ranked.iter().any(|a| a.label == "web/util.js"),
            "generated files drop out: {ranked:?}"
        );
    }

    #[test]
    fn sleep_windows_churn_so_history_stops_outranking_the_present() {
        let src = tempfile::tempdir().unwrap();
        fs::create_dir_all(src.path().join("src")).unwrap();
        fs::write(src.path().join("src/busy.rs"), "pub fn b() {}\n").unwrap();
        fs::write(src.path().join("src/quiet.rs"), "pub fn q() {}\n").unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let store = brain_store::Store::open(store_dir.path()).unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        // busy.rs churns hard before the sleep.
        for i in 0..6 {
            std::thread::sleep(std::time::Duration::from_millis(2));
            fs::write(
                src.path().join("src/busy.rs"),
                format!("pub fn b() {{ /* {i} */ }}\n"),
            )
            .unwrap();
            refresh(&store, src.path(), "twin/app").unwrap();
        }
        crate::sleep::sleep(&store, "twin/app").unwrap();

        // After the sleep, one small edit to quiet.rs outranks all of
        // busy.rs's history: 1 recent (4) + lifetime 2 = 6 beats
        // busy.rs's 0 recent + lifetime capped 7 = 7... so make it two.
        for i in 0..2 {
            std::thread::sleep(std::time::Duration::from_millis(2));
            fs::write(
                src.path().join("src/quiet.rs"),
                format!("pub fn q() {{ /* {i} */ }}\n"),
            )
            .unwrap();
            refresh(&store, src.path(), "twin/app").unwrap();
        }
        let mut index = MemIndex::new();
        replay(&store, &mut index).unwrap();
        let ranked = attend(&store, &index, "twin/app").unwrap();
        let busy = ranked
            .iter()
            .find(|a| a.label == "src/busy.rs")
            .expect("busy ranked");
        let quiet = ranked
            .iter()
            .find(|a| a.label == "src/quiet.rs")
            .expect("quiet ranked");
        assert!(
            quiet.score > busy.score,
            "recent work outranks history: quiet {} vs busy {}",
            quiet.score,
            busy.score
        );
        assert!(
            busy.reasons.iter().any(|r| r.contains("(0 recent)")),
            "{:?}",
            busy.reasons
        );
    }
}
