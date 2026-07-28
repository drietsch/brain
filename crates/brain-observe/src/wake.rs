//! Wake: one command, the whole present.
//!
//! The counterpart of `sleep` — compose the last consolidated summary,
//! what changed since it, where attention points, what is stale enough to
//! matter, and what is in flight (active plans, pending changes,
//! unfinished features) into a single token-budgeted orientation. A fresh
//! session runs `brain wake <prefix>` instead of spelunking the repo;
//! nothing here is stored, everything is a query (ADR-009, ADR-016).

use crate::sleep::delta_since;
use crate::twin::{self, latest, latest_at, Severity};
use brain_core::ids::StableId;
use brain_core::object::Object;
use brain_index::{Index, MemIndex};
use brain_store::{now_ms, Store, StoreError};
use std::collections::BTreeSet;
use std::fmt::Write as _;

/// Render the orientation. `full` lifts the per-section caps.
pub fn wake(
    store: &Store,
    index: &MemIndex,
    prefix: &str,
    full: bool,
) -> Result<String, StoreError> {
    let insights = twin::insights_with(store, index, prefix)?;
    let ranked = crate::attention::attend_with(store, index, prefix, &insights)?;
    wake_with(store, index, prefix, full, &insights, &ranked)
}

/// Render wake from shared derived projections. This is semantically
/// identical to [`wake`] but avoids recomputing insights and attention when
/// a human surface needs the same material in structured and textual forms.
pub fn wake_with(
    store: &Store,
    index: &MemIndex,
    prefix: &str,
    full: bool,
    ins: &twin::Insights,
    ranked: &[crate::attention::Attention],
) -> Result<String, StoreError> {
    let cap = if full { usize::MAX } else { 5 };
    let now = now_ms();
    let mut out = String::new();
    let repo_sid = StableId::derive(&["repo", prefix]);
    writeln!(out, "== wake: {prefix} ==").ok();

    let since: u64 = latest(index, store, &repo_sid, "consolidated_until")?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    match latest_at(index, store, &repo_sid, "session_summary")? {
        Some((at, summary)) => {
            writeln!(out, "last sleep {}: {summary}", age(now, at)).ok();
        }
        None => {
            writeln!(out, "never slept — everything below counts as new").ok();
        }
    }

    let delta = delta_since(store, index, prefix, since)?;
    writeln!(
        out,
        "since then: {} added, {} changed file(s); {} doc update(s); {} protocol(s){}; {} note(s)",
        delta.added.len(),
        delta.changed.len(),
        delta.doc_updates,
        delta.new_runs,
        delta.verdict,
        delta.notes
    )
    .ok();
    if let (Some(branch), Some(commit)) = (&ins.git_branch, &ins.git_commit) {
        writeln!(out, "git: {branch} @ {}", &commit[..commit.len().min(12)]).ok();
    }

    if !ins.failing.is_empty() {
        writeln!(out, "FAILING: {} test case(s)", ins.failing.len()).ok();
        for name in ins.failing.iter().take(cap.min(3)) {
            writeln!(out, "  ✗ {name}").ok();
        }
    }

    if !ranked.is_empty() {
        writeln!(out, "attention:").ok();
        for a in ranked.iter().take(cap) {
            writeln!(
                out,
                "  {:>3}  {} ({})",
                a.score,
                a.label,
                a.reasons.join(", ")
            )
            .ok();
        }
    }

    let warns: Vec<_> = ins
        .stale_docs
        .iter()
        .filter(|d| d.severity == Severity::Warn)
        .collect();
    let infos = ins.stale_docs.len() - warns.len();
    if !warns.is_empty() || infos > 0 {
        writeln!(
            out,
            "stale: {} warn, {infos} info — `brain twin stale {prefix}`",
            warns.len()
        )
        .ok();
        for d in warns.iter().take(cap.min(3)) {
            writeln!(
                out,
                "  [warn] {} ({}): {}",
                d.slug,
                d.kind,
                d.changed.join(", ")
            )
            .ok();
        }
    }

    // In-flight: active plans, unsettled governed changes, open features.
    let mut inflight: Vec<String> = Vec::new();
    for (slug, title) in ins.plans.iter().take(cap) {
        inflight.push(format!("plan {slug}: {title}"));
    }
    let mut seen = BTreeSet::new();
    for node in index.entities_by_kind("change") {
        let Ok(Object::Entity { id, labels, .. }) = store.get(&node) else {
            continue;
        };
        if labels.get("prefix").map(String::as_str) != Some(prefix) || !seen.insert(id.clone()) {
            continue;
        }
        if let Some(status) = latest(index, store, &id, "status")? {
            if ["proposed", "applied", "indeterminate", "broken"].contains(&status.as_str()) {
                let slug = labels.get("slug").cloned().unwrap_or_default();
                inflight.push(format!("change {slug} [{status}]"));
            }
        }
    }
    for feature in &ins.features {
        if !feature.done {
            let counted = if feature.by_parts { "parts" } else { "DoD" };
            inflight.push(format!(
                "feature {} [{}] {counted} {}",
                feature.slug, feature.status, feature.fraction
            ));
        }
    }
    if !inflight.is_empty() {
        writeln!(out, "in flight ({}):", inflight.len()).ok();
        for item in inflight.iter().take(cap) {
            writeln!(out, "  {item}").ok();
        }
        if inflight.len() > cap {
            writeln!(out, "  … {} more", inflight.len() - cap).ok();
        }
    }

    let fresh_notes: Vec<_> = ins.notes.iter().filter(|(at, _, _)| *at > since).collect();
    if !fresh_notes.is_empty() {
        writeln!(out, "notes since sleep:").ok();
        for (at, entity, text) in fresh_notes.iter().take(cap) {
            writeln!(out, "  [{}] {entity}: {text}", age(now, *at)).ok();
        }
    }

    let findings = crate::coherence::check(store, index, prefix)?;
    if !findings.is_empty() {
        writeln!(out, "coherence ({} finding(s)):", findings.len()).ok();
        for f in findings.iter().take(cap.min(3)) {
            writeln!(out, "  {f}").ok();
        }
    }

    write!(
        out,
        "next: brain attend {prefix} | brain twin stale {prefix} | brain sleep {prefix} before you go"
    )
    .ok();
    Ok(out)
}

fn age(now: u64, at: u64) -> String {
    let s = now.saturating_sub(at) / 1000;
    if s >= 86_400 {
        format!("{}d ago", s / 86_400)
    } else if s >= 3_600 {
        format!("{}h ago", s / 3_600)
    } else if s >= 60 {
        format!("{}m ago", s / 60)
    } else {
        format!("{s}s ago")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::twin::refresh;
    use brain_index::replay;
    use std::fs;

    #[test]
    fn wake_composes_a_truthful_orientation() {
        let src = tempfile::tempdir().unwrap();
        fs::create_dir_all(src.path().join("src")).unwrap();
        fs::create_dir_all(src.path().join("docs/plans")).unwrap();
        fs::write(src.path().join("src/main.rs"), "pub fn main() {}\n").unwrap();
        fs::write(
            src.path().join("docs/plans/build-x.md"),
            "# Build X\n\nsrc/main.rs.\n",
        )
        .unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let store = brain_store::Store::open(store_dir.path()).unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        crate::sleep::sleep(&store, "twin/app").unwrap();

        // Post-sleep activity: one edit, one note.
        std::thread::sleep(std::time::Duration::from_millis(5));
        fs::write(
            src.path().join("src/main.rs"),
            "pub fn main() { /* v2 */ }\n",
        )
        .unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        crate::twin::add_note(
            &store,
            &StableId::derive(&["repo", "twin/app"]),
            "picked up where we left off",
        )
        .unwrap();

        let mut index = MemIndex::new();
        replay(&store, &mut index).unwrap();
        let text = wake(&store, &index, "twin/app", false).unwrap();
        assert!(text.contains("last sleep"), "{text}");
        assert!(text.contains("0 added, 1 changed file(s)"), "{text}");
        assert!(
            text.contains("plan build-x"),
            "active plan in flight: {text}"
        );
        assert!(text.contains("picked up where we left off"), "{text}");
        assert!(
            text.lines().count() <= 40,
            "budgeted: {} lines",
            text.lines().count()
        );

        // A finished plan leaves the in-flight list.
        crate::lifecycle::set(
            &store,
            &index,
            &StableId::derive(&["plan", "twin/app", "build-x"]),
            crate::lifecycle::Lifecycle::Done,
            None,
        )
        .unwrap();
        let mut index = MemIndex::new();
        replay(&store, &mut index).unwrap();
        let text = wake(&store, &index, "twin/app", false).unwrap();
        assert!(!text.contains("plan build-x"), "{text}");

        // Determinism: identical recompute (age strings share the same second).
        assert_eq!(text, wake(&store, &index, "twin/app", false).unwrap());
    }
}
