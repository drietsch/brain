//! Next: the work queue — what should happen now, and why.
//!
//! The past has sessions and protocols, the present has wake and attend;
//! this is the future leg: everything the graph knows is unfinished or
//! wrong, ranked so an agent (or the orchestrating developer) can pick
//! the top of one queue instead of assembling it from five reports.
//! Nothing here is stored; data first ([`NextItem`]), text second
//! ([`render`]).

use crate::twin::{self, latest, Severity};
use brain_core::object::Object;
use brain_index::{Index, MemIndex};
use brain_store::{Store, StoreError};
use serde::Serialize;
use std::collections::BTreeSet;
use std::fmt::Write as _;

#[derive(Serialize, Debug)]
pub struct NextItem {
    pub score: u32,
    /// failing_test | change | stale_doc | feature_gap | coherence | plan
    pub kind: String,
    pub label: String,
    pub why: String,
    /// The command that acts on it.
    pub via: String,
}

/// Compose the ranked queue. Scores are category weights, not
/// measurements: broken beats stale beats unfinished beats open-ended.
pub fn queue(store: &Store, index: &MemIndex, prefix: &str) -> Result<Vec<NextItem>, StoreError> {
    let ins = twin::insights_with(store, index, prefix)?;
    let mut items: Vec<NextItem> = Vec::new();

    for name in &ins.failing {
        items.push(NextItem {
            score: 100,
            kind: "failing_test".into(),
            label: name.clone(),
            why: "latest recorded result is fail".into(),
            via: format!("brain twin tests {prefix}"),
        });
    }

    // Governed changes that are not settled. Indeterminate and broken
    // demand reconciliation; proposed is waiting for a decision.
    let mut seen = BTreeSet::new();
    for node in index.entities_by_kind("change") {
        let Ok(Object::Entity { id, labels, .. }) = store.get(&node) else {
            continue;
        };
        if labels.get("prefix").map(String::as_str) != Some(prefix) || !seen.insert(id.clone()) {
            continue;
        }
        let Some(status) = latest(index, store, &id, "status")? else {
            continue;
        };
        let slug = labels.get("slug").cloned().unwrap_or_default();
        match status.as_str() {
            "indeterminate" | "broken" => items.push(NextItem {
                score: 90,
                kind: "change".into(),
                label: slug.clone(),
                why: format!("governed change is {status} — reconcile before retrying"),
                via: format!("brain change show {prefix} {slug}"),
            }),
            // Applied is unfinished: the file was written but the tests
            // have not vouched for it. Skipping it here once let the
            // queue read empty while Work counted four changes waiting.
            "applied" => items.push(NextItem {
                score: 65,
                kind: "change".into(),
                label: slug.clone(),
                why: "applied, but the tests have not vouched for it".into(),
                via: format!("brain change verify {prefix} {slug}"),
            }),
            "proposed" => items.push(NextItem {
                score: 55,
                kind: "change".into(),
                label: slug.clone(),
                why: "proposed and waiting".into(),
                via: format!("brain change apply {prefix} {slug} --cap fs"),
            }),
            _ => {}
        }
    }

    for d in &ins.stale_docs {
        if d.severity != Severity::Warn {
            continue;
        }
        let ack = match d.kind.as_str() {
            "decision" => format!("brain adr ack {prefix} {}", d.slug),
            "plan" => format!("brain plan ack {prefix} {}", d.slug),
            _ => format!("brain artifact ack {prefix} {} {}", d.kind, d.slug),
        };
        items.push(NextItem {
            score: 70,
            kind: "stale_doc".into(),
            label: d.slug.clone(),
            why: format!("{} changed after the doc", d.changed.join(", ")),
            via: format!("fix the doc, or if still accurate: {ack}"),
        });
    }

    for feature in &ins.features {
        if feature.done {
            continue;
        }
        let report = crate::features::evaluate(store, index, prefix, &feature.slug)?;
        let why = if report.by_parts() {
            match &report.blocked_by {
                Some(part) => format!("waiting on part {part} ({})", feature.fraction),
                None => format!("parts incomplete ({})", feature.fraction),
            }
        } else {
            let missing: Vec<&str> = report
                .checks
                .iter()
                .filter(|c| c.count == 0)
                .map(|c| c.predicate.as_str())
                .collect();
            if missing.is_empty() {
                continue;
            }
            format!("missing {}", missing.join(", "))
        };
        items.push(NextItem {
            score: 60,
            kind: "feature_gap".into(),
            label: feature.slug.clone(),
            why,
            via: format!("brain feature link {prefix} {} <predicate> <target>", feature.slug),
        });
    }

    for finding in crate::coherence::check(store, index, prefix)? {
        items.push(NextItem {
            score: 50,
            kind: "coherence".into(),
            label: finding.to_string(),
            why: "the graph claims it; nothing observed corroborates it".into(),
            via: format!("brain spine {prefix}"),
        });
    }

    for (slug, title) in &ins.plans {
        items.push(NextItem {
            score: 40,
            kind: "plan".into(),
            label: format!("{slug}: {title}"),
            why: "active plan".into(),
            via: format!("brain plan done {prefix} {slug} when finished"),
        });
    }

    items.sort_by(|a, b| b.score.cmp(&a.score).then(a.label.cmp(&b.label)));
    Ok(items)
}

/// The textual projection of the queue.
pub fn render(items: &[NextItem], prefix: &str, top: usize) -> String {
    let mut out = String::new();
    writeln!(out, "== next: {prefix} ==").ok();
    if items.is_empty() {
        write!(
            out,
            "queue is empty — nothing failing, nothing stale, nothing unfinished"
        )
        .ok();
        return out;
    }
    for (i, item) in items.iter().take(top).enumerate() {
        writeln!(
            out,
            "{:>2}. [{:>3}] {} {} — {}",
            i + 1,
            item.score,
            item.kind,
            item.label,
            item.why
        )
        .ok();
        writeln!(out, "        via: {}", item.via).ok();
    }
    if items.len() > top {
        writeln!(out, "  … {} more (--top {})", items.len() - top, items.len()).ok();
    }
    write!(out, "the queue is derived — act on it, refresh, and it re-ranks").ok();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::twin::refresh;
    use brain_index::replay;
    use std::fs;

    /// An applied change is unfinished until the tests vouch for it —
    /// skipping it once let the queue read empty while the work surface
    /// counted four changes waiting.
    #[test]
    fn an_applied_change_stays_in_the_queue_until_verified() {
        let src = tempfile::tempdir().unwrap();
        fs::create_dir_all(src.path().join("src")).unwrap();
        fs::write(src.path().join("src/main.rs"), "pub fn main() {}\n").unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let store = brain_store::Store::open(store_dir.path()).unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();

        let proposal = crate::govern::propose(
            &store,
            src.path(),
            "twin/app",
            "src/main.rs",
            "pub fn main() { /* v2 */ }\n",
            "tighten",
        )
        .unwrap();
        crate::govern::apply(
            &store,
            src.path(),
            "twin/app",
            &proposal.slug,
            &["fs".to_string()],
        )
        .unwrap();

        let mut index = MemIndex::new();
        replay(&store, &mut index).unwrap();
        let items = queue(&store, &index, "twin/app").unwrap();
        let row = items
            .iter()
            .find(|i| i.kind == "change")
            .expect("the applied change is queued: {items:?}");
        assert!(row.why.contains("vouched"), "{}", row.why);
        assert!(row.via.contains("change verify"), "{}", row.via);
        assert_eq!(row.score, 65);
    }

    #[test]
    fn queue_ranks_rot_above_open_plans() {
        let src = tempfile::tempdir().unwrap();
        fs::create_dir_all(src.path().join("src")).unwrap();
        fs::create_dir_all(src.path().join("docs/plans")).unwrap();
        fs::write(src.path().join("src/main.rs"), "pub fn main() {}\n").unwrap();
        fs::write(
            src.path().join("docs/guide.md"),
            "# Guide\n\nHow src/main.rs works.\n",
        )
        .unwrap();
        fs::write(
            src.path().join("docs/plans/build-x.md"),
            "# Build X\n\nsrc/main.rs.\n",
        )
        .unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let store = brain_store::Store::open(store_dir.path()).unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();

        // The guide rots: its mentioned file changes after it was captured.
        std::thread::sleep(std::time::Duration::from_millis(5));
        fs::write(
            src.path().join("src/main.rs"),
            "pub fn main() { /* v2 */ }\n",
        )
        .unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();

        let mut index = MemIndex::new();
        replay(&store, &mut index).unwrap();
        let items = queue(&store, &index, "twin/app").unwrap();

        let stale = items.iter().position(|i| i.kind == "stale_doc");
        let plan = items.iter().position(|i| i.kind == "plan");
        assert!(stale.is_some(), "stale guide queued: {items:?}");
        assert!(plan.is_some(), "active plan queued: {items:?}");
        assert!(stale < plan, "rot outranks open-ended work: {items:?}");

        let text = render(&items, "twin/app", 10);
        assert!(text.contains("stale_doc guide"), "{text}");
        assert!(text.contains("via:"), "{text}");

        // The same queue, as data.
        let v = serde_json::to_value(&items).unwrap();
        assert!(v.as_array().unwrap().len() >= 2);
    }
}
