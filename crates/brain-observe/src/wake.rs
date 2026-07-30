//! Wake: one command, the whole present.
//!
//! The counterpart of `sleep` — compose the last consolidated summary,
//! what changed since it, where attention points, what is stale enough to
//! matter, and what is in flight (active plans, pending changes,
//! unfinished features) into a single token-budgeted orientation. A fresh
//! session runs `brain wake <prefix>` instead of spelunking the repo;
//! nothing here is stored, everything is a query (ADR-009, ADR-016).
//!
//! The orientation is data first ([`Orientation`], serializable for
//! `--json` and other consumers) and text second ([`render`]).

use crate::sleep::delta_since;
use crate::twin::{self, latest, latest_at, Severity};
use brain_core::ids::StableId;
use brain_core::object::Object;
use brain_index::{Index, MemIndex};
use brain_store::{now_ms, Store, StoreError};
use serde::Serialize;
use std::collections::BTreeSet;
use std::fmt::Write as _;

/// How the working tree relates to what the twin last observed. `None`
/// upstream means the twin never recorded where it looked (older stores).
#[derive(Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum TreeDrift {
    InSync,
    Ahead {
        added: Vec<String>,
        changed: Vec<String>,
        deleted: Vec<String>,
    },
    /// The recorded root no longer exists on this machine.
    Unavailable { root: String },
}

/// Compare the working tree at the twin's recorded root against the graph,
/// read-only. This is what makes wake honest about uncommitted work: the
/// graph only learns on refresh, but it can still say what it has not seen.
pub fn tree_drift(
    store: &Store,
    index: &MemIndex,
    prefix: &str,
) -> Result<Option<TreeDrift>, StoreError> {
    let repo_sid = StableId::derive(&["repo", prefix]);
    let Some(root) = latest(index, store, &repo_sid, "root")? else {
        return Ok(None);
    };
    let path = std::path::Path::new(&root);
    if !path.is_dir() {
        return Ok(Some(TreeDrift::Unavailable { root }));
    }
    let report = twin::status(store, path, prefix)?;
    if report.added.is_empty() && report.changed.is_empty() && report.deleted.is_empty() {
        Ok(Some(TreeDrift::InSync))
    } else {
        Ok(Some(TreeDrift::Ahead {
            added: report.added,
            changed: report.changed,
            deleted: report.deleted,
        }))
    }
}

/// The whole present, as data: everything `brain wake` says, uncapped.
/// Rendering truncates; the data never does.
#[derive(Serialize)]
pub struct Orientation {
    pub prefix: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_sleep: Option<Sleep>,
    pub since: Delta,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git: Option<Git>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_tree: Option<TreeDrift>,
    pub failing: Vec<String>,
    pub attention: Vec<AttentionRow>,
    pub stale: Stale,
    pub in_flight: Vec<InFlight>,
    pub notes_since_sleep: Vec<Note>,
    pub coherence: Vec<String>,
}

#[derive(Serialize)]
pub struct Sleep {
    pub at_ms: u64,
    pub summary: String,
}

#[derive(Serialize)]
pub struct Delta {
    pub since_ms: u64,
    pub added: usize,
    pub changed: usize,
    pub doc_updates: usize,
    pub protocols: usize,
    pub verdict: String,
    pub notes: usize,
}

#[derive(Serialize)]
pub struct Git {
    pub branch: String,
    pub commit: String,
}

#[derive(Serialize)]
pub struct AttentionRow {
    pub score: u32,
    pub label: String,
    pub kind: String,
    pub reasons: Vec<String>,
}

#[derive(Serialize)]
pub struct Stale {
    pub warn: usize,
    pub info: usize,
    pub warn_docs: Vec<StaleDocRow>,
}

#[derive(Serialize)]
pub struct StaleDocRow {
    pub slug: String,
    pub kind: String,
    pub changed: Vec<String>,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InFlight {
    Plan {
        slug: String,
        title: String,
    },
    Change {
        slug: String,
        status: String,
    },
    Feature {
        slug: String,
        status: String,
        counted: String,
        fraction: String,
    },
}

#[derive(Serialize)]
pub struct Note {
    pub at_ms: u64,
    pub entity: String,
    pub text: String,
}

/// Compose the orientation from the graph (and the working tree, when the
/// twin recorded where it looked).
pub fn orientation(
    store: &Store,
    index: &MemIndex,
    prefix: &str,
) -> Result<Orientation, StoreError> {
    let ins = twin::insights_with(store, index, prefix)?;
    let ranked = crate::attention::attend_with(store, index, prefix, &ins)?;
    let drift = tree_drift(store, index, prefix)?;
    let repo_sid = StableId::derive(&["repo", prefix]);

    let since: u64 = latest(index, store, &repo_sid, "consolidated_until")?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let last_sleep = latest_at(index, store, &repo_sid, "session_summary")?
        .map(|(at, summary)| Sleep { at_ms: at, summary });

    let delta = delta_since(store, index, prefix, since)?;
    let since = Delta {
        since_ms: since,
        added: delta.added.len(),
        changed: delta.changed.len(),
        doc_updates: delta.doc_updates,
        protocols: delta.new_runs,
        verdict: delta.verdict,
        notes: delta.notes,
    };

    let git = match (&ins.git_branch, &ins.git_commit) {
        (Some(branch), Some(commit)) => Some(Git {
            branch: branch.clone(),
            commit: commit.clone(),
        }),
        _ => None,
    };

    let attention = ranked
        .iter()
        .map(|a| AttentionRow {
            score: a.score,
            label: a.label.clone(),
            kind: a.kind.clone(),
            reasons: a.reasons.clone(),
        })
        .collect();

    let warn_docs: Vec<StaleDocRow> = ins
        .stale_docs
        .iter()
        .filter(|d| d.severity == Severity::Warn)
        .map(|d| StaleDocRow {
            slug: d.slug.clone(),
            kind: d.kind.clone(),
            changed: d.changed.clone(),
        })
        .collect();
    let stale = Stale {
        warn: warn_docs.len(),
        info: ins.stale_docs.len() - warn_docs.len(),
        warn_docs,
    };

    // In-flight: active plans, unsettled governed changes, open features.
    let mut in_flight: Vec<InFlight> = Vec::new();
    for (slug, title) in &ins.plans {
        in_flight.push(InFlight::Plan {
            slug: slug.clone(),
            title: title.clone(),
        });
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
                in_flight.push(InFlight::Change {
                    slug: labels.get("slug").cloned().unwrap_or_default(),
                    status,
                });
            }
        }
    }
    for feature in &ins.features {
        if !feature.done {
            in_flight.push(InFlight::Feature {
                slug: feature.slug.clone(),
                status: feature.status.clone(),
                counted: if feature.by_parts { "parts" } else { "DoD" }.to_string(),
                fraction: feature.fraction.clone(),
            });
        }
    }

    let notes_since_sleep = ins
        .notes
        .iter()
        .filter(|(at, _, _)| *at > since.since_ms)
        .map(|(at, entity, text)| Note {
            at_ms: *at,
            entity: entity.clone(),
            text: text.clone(),
        })
        .collect();

    let coherence = crate::coherence::check(store, index, prefix)?
        .iter()
        .map(|f| f.to_string())
        .collect();

    Ok(Orientation {
        prefix: prefix.to_string(),
        last_sleep,
        since,
        git,
        working_tree: drift,
        failing: ins.failing.clone(),
        attention,
        stale,
        in_flight,
        notes_since_sleep,
        coherence,
    })
}

/// Render the orientation. `full` lifts the per-section caps.
pub fn wake(
    store: &Store,
    index: &MemIndex,
    prefix: &str,
    full: bool,
) -> Result<String, StoreError> {
    Ok(render(&orientation(store, index, prefix)?, full))
}

/// The textual projection of an [`Orientation`].
pub fn render(o: &Orientation, full: bool) -> String {
    let cap = if full { usize::MAX } else { 5 };
    let now = now_ms();
    let mut out = String::new();
    writeln!(out, "== wake: {} ==", o.prefix).ok();

    match &o.last_sleep {
        Some(s) => {
            writeln!(out, "last sleep {}: {}", age(now, s.at_ms), s.summary).ok();
        }
        None => {
            writeln!(out, "never slept — everything below counts as new").ok();
        }
    }

    writeln!(
        out,
        "since then: {} added, {} changed file(s); {} doc update(s); {} protocol(s){}; {} note(s)",
        o.since.added, o.since.changed, o.since.doc_updates, o.since.protocols, o.since.verdict,
        o.since.notes
    )
    .ok();
    if let Some(git) = &o.git {
        writeln!(
            out,
            "git: {} @ {}",
            git.branch,
            &git.commit[..git.commit.len().min(12)]
        )
        .ok();
    }
    match &o.working_tree {
        Some(TreeDrift::InSync) => {
            writeln!(out, "working tree: in sync with the twin").ok();
        }
        Some(TreeDrift::Ahead {
            added,
            changed,
            deleted,
        }) => {
            writeln!(
                out,
                "working tree ahead of twin: {} added, {} changed, {} deleted — answers about these files may be stale",
                added.len(),
                changed.len(),
                deleted.len()
            )
            .ok();
            for path in changed.iter().chain(added).chain(deleted).take(cap.min(3)) {
                writeln!(out, "  ~ {path}").ok();
            }
        }
        Some(TreeDrift::Unavailable { root }) => {
            writeln!(out, "working tree: {root} not found — drift unknown").ok();
        }
        None => {}
    }

    if !o.failing.is_empty() {
        writeln!(out, "FAILING: {} test case(s)", o.failing.len()).ok();
        for name in o.failing.iter().take(cap.min(3)) {
            writeln!(out, "  ✗ {name}").ok();
        }
    }

    if !o.attention.is_empty() {
        writeln!(out, "attention:").ok();
        for a in o.attention.iter().take(cap) {
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

    if o.stale.warn + o.stale.info > 0 {
        writeln!(
            out,
            "stale: {} warn, {} info — `brain twin stale {}`",
            o.stale.warn, o.stale.info, o.prefix
        )
        .ok();
        for d in o.stale.warn_docs.iter().take(cap.min(3)) {
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

    if !o.in_flight.is_empty() {
        writeln!(out, "in flight ({}):", o.in_flight.len()).ok();
        for item in o.in_flight.iter().take(cap) {
            let line = match item {
                InFlight::Plan { slug, title } => format!("plan {slug}: {title}"),
                InFlight::Change { slug, status } => format!("change {slug} [{status}]"),
                InFlight::Feature {
                    slug,
                    status,
                    counted,
                    fraction,
                } => format!("feature {slug} [{status}] {counted} {fraction}"),
            };
            writeln!(out, "  {line}").ok();
        }
        if o.in_flight.len() > cap {
            writeln!(out, "  … {} more", o.in_flight.len() - cap).ok();
        }
    }

    if !o.notes_since_sleep.is_empty() {
        writeln!(out, "notes since sleep:").ok();
        for n in o.notes_since_sleep.iter().take(cap) {
            writeln!(out, "  [{}] {}: {}", age(now, n.at_ms), n.entity, n.text).ok();
        }
    }

    if !o.coherence.is_empty() {
        writeln!(out, "coherence ({} finding(s)):", o.coherence.len()).ok();
        for f in o.coherence.iter().take(cap.min(3)) {
            writeln!(out, "  {f}").ok();
        }
    }

    write!(
        out,
        "next: brain attend {p} | brain twin stale {p} | brain sleep {p} before you go",
        p = o.prefix
    )
    .ok();
    out
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
        assert!(text.contains("working tree: in sync with the twin"), "{text}");
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

        // The same present, as data: the JSON projection carries the
        // uncapped structure the text renders.
        let o = orientation(&store, &index, "twin/app").unwrap();
        let v = serde_json::to_value(&o).unwrap();
        assert_eq!(v["prefix"], "twin/app");
        assert_eq!(v["working_tree"]["state"], "in_sync");
        assert_eq!(v["in_flight"][0]["kind"], "plan");
        assert_eq!(v["in_flight"][0]["slug"], "build-x");
        assert!(v["last_sleep"]["at_ms"].as_u64().unwrap() > 0);

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

        // Uncommitted work: the tree drifts and wake says so — the graph
        // only learns on refresh, but it can still name what it has not seen.
        fs::write(
            src.path().join("src/main.rs"),
            "pub fn main() { /* v3 */ }\n",
        )
        .unwrap();
        let text = wake(&store, &index, "twin/app", false).unwrap();
        assert!(
            text.contains("working tree ahead of twin: 0 added, 1 changed, 0 deleted"),
            "{text}"
        );
        assert!(text.contains("~ src/main.rs"), "{text}");
    }
}
