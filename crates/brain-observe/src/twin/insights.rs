//! What the twin concluded from what it observed — churn, hubs, coverage, quality, rot.

use super::*;
use crate::testing;
use brain_core::ids::{NodeId, StableId};
use brain_core::object::Object;
use brain_index::{replay, Index, MemIndex};
use brain_store::{now_ms, Store, StoreError};
use std::collections::{BTreeMap, BTreeSet};

/// How far along one feature is, counted in whichever terms it is judged.
///
/// A feature with parts is judged by its parts (ADR-028), so the fraction
/// has to come from `DoneReport::score` rather than from the feature's own
/// links. Reading the score off the link count made every parent report
/// what it happened to be linked to directly — the root of the spine said
/// `1/4` while its thirteen parts were all ready.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FeatureProgress {
    pub slug: String,
    pub status: String,
    /// "3/4" of requirements, or "2/13" of parts.
    pub fraction: String,
    pub done: bool,
    /// Whether the fraction counts parts rather than requirements.
    pub by_parts: bool,
}

/// One quality reading of the codebase at a moment: what the tests said,
/// how many features are ready, how many documents drifted, and how many
/// claims nothing corroborates. Complete or absent, never partial — all
/// six numbers land together, so a reader never sees a half-measured
/// moment. Points accrue only when a refresh or sleep found the numbers
/// moved; the series is bounded by change, not by time.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct QualityPoint {
    pub at_ms: u64,
    /// (passed, total) of the latest imported run; None until a run exists.
    pub tests: Option<(usize, usize)>,
    pub features_done: usize,
    pub features_total: usize,
    pub stale_warnings: usize,
    pub uncorroborated: usize,
}

#[derive(Debug, Default, serde::Serialize)]
pub struct Insights {
    pub files: usize,
    pub deleted_files: usize,
    pub symbols: usize,
    pub relations: usize,
    /// External dependencies (unresolved imports): (module, importer count).
    pub external_modules: Vec<(String, usize)>,
    /// Most-edited files since twinning: (path, observed versions > 1).
    pub churn: Vec<(String, usize)>,
    /// Most-imported files: (path, importer count).
    pub hubs: Vec<(String, usize)>,
    /// Largest files by declared symbols: (path, symbol count).
    pub largest: Vec<(String, usize)>,
    /// Most recent agent notes: (at_ms, entity path, text), newest first.
    pub notes: Vec<(u64, String, String)>,
    pub git_commit: Option<String>,
    pub git_branch: Option<String>,
    /// Growth series from the repo entity, oldest first: (at_ms, files,
    /// symbols, relations) — one point per refresh that changed the totals.
    pub series: Vec<(u64, usize, usize, usize)>,
    /// Quality series from the repo entity, oldest first — one point per
    /// refresh or sleep that found a quality number moved.
    pub quality: Vec<QualityPoint>,
    /// Decisions (ADRs) under the prefix, newest first: (slug, title, status).
    pub decisions: Vec<(String, String, String)>,
    /// Plans under the prefix, newest first: (slug, title).
    pub plans: Vec<(String, String)>,
    /// Files a decision mentions — hotspots with documented rationale.
    pub decided: BTreeSet<String>,
    /// Agent skills under the prefix: (slug, agent, description-or-name).
    pub skills: Vec<(String, String, String)>,
    /// Agent configuration under the prefix: (slug, agent, role).
    pub agent_configs: Vec<(String, String, String)>,
    /// Documents that fail their template's contract: (slug, kind, missing).
    pub nonconforming: Vec<(String, String, String)>,
    /// Features under the prefix, each judged in its own terms.
    pub features: Vec<FeatureProgress>,
    /// Test files (by role) and total declared test cases across all files.
    pub test_files: usize,
    pub tests_declared: usize,
    /// Latest imported run: (at_ms, total, passed, failed).
    pub last_run: Option<(u64, usize, usize, usize)>,
    /// Test cases whose latest recorded result is `fail`.
    pub failing: Vec<String>,
    /// Most-imported files with no declared tests and no covering test file.
    pub untested_hubs: Vec<(String, usize)>,
    /// Docs whose mentioned files changed after the doc was last updated
    /// or acknowledged. Derived at query time, never written — stale is a
    /// judgment about now, not a fact about then. Only active documents
    /// rot, only live mentions count, and severity follows the kind's rot
    /// policy.
    pub stale_docs: Vec<StaleDoc>,
    /// Artifacts of graph-defined kinds (capture rules): (kind, count).
    pub custom_artifacts: Vec<(String, usize)>,
}

/// How loudly a stale document should speak. Warn = the doc describes
/// the present and is now wrong somewhere; info = a record whose context
/// moved on — visible, never nagging.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Warn,
    Info,
}

impl Severity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Severity::Warn => "warn",
            Severity::Info => "info",
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct StaleDoc {
    pub slug: String,
    pub kind: String,
    pub severity: Severity,
    /// Live-mentioned files that changed after the doc's effective time.
    pub changed: Vec<String>,
}

/// The rot policy for a kind: `None` = exempt, else the severity stale
/// docs of this kind carry. The registry's `rot` value (graph over
/// compiled defaults: none|info|warn) with code fallbacks — decisions and
/// plans are records once written (info); skills, agent config, and
/// taught kinds describe the present (warn).
pub fn rot_severity(rot: &str, kind: &str) -> Option<Severity> {
    match rot {
        "none" => None,
        "info" => Some(Severity::Info),
        "warn" => Some(Severity::Warn),
        _ => match kind {
            "decision" | "plan" => Some(Severity::Info),
            _ => Some(Severity::Warn),
        },
    }
}

/// Record that an agent reviewed an artifact against the present. The
/// observation's timestamp resets the staleness clock without touching
/// any file. Deliberately unguarded — re-acknowledging is the point.
pub fn ack(store: &Store, sid: &StableId, note: &str) -> Result<NodeId, StoreError> {
    observe_src(store, sid, "reviewed", note, "agent", now_ms())
}

pub fn insights(store: &Store, prefix: &str) -> Result<Insights, StoreError> {
    let mut index = MemIndex::new();
    replay(store, &mut index)?;
    insights_with(store, &index, prefix)
}

pub fn insights_with(
    store: &Store,
    index: &MemIndex,
    prefix: &str,
) -> Result<Insights, StoreError> {
    let mut ins = Insights::default();
    let ns = store.namespace()?;
    let mut file_sids: Vec<(String, StableId)> = Vec::new();

    for (name, node) in &ns {
        let Some(rel) = name.strip_prefix(&format!("{prefix}/")) else {
            continue;
        };
        if let Ok(Object::Entity {
            id, entity_kind, ..
        }) = store.get(node)
        {
            if entity_kind == "source_file" {
                file_sids.push((rel.to_string(), id));
            }
        }
    }

    // Decisions and plans under this prefix, newest first by content time.
    let mut decision_sids: BTreeSet<StableId> = BTreeSet::new();
    let mut decisions: Vec<(u64, String, String, String)> = Vec::new();
    let mut plans: Vec<(u64, String, String)> = Vec::new();
    for kind in ["decision", "plan"] {
        let mut seen: BTreeSet<StableId> = BTreeSet::new();
        for node in index.entities_by_kind(kind) {
            let Ok(Object::Entity { id, labels, .. }) = store.get(&node) else {
                continue;
            };
            if labels.get("prefix").map(String::as_str) != Some(prefix) || !seen.insert(id.clone())
            {
                continue;
            }
            if !crate::lifecycle::of(index, store, &id)?.0.is_active() {
                continue; // superseded/done/retired documents are history
            }
            let slug = labels.get("slug").cloned().unwrap_or_default();
            let title = latest(index, store, &id, "title")?
                .or_else(|| labels.get("title").cloned())
                .unwrap_or_else(|| slug.clone());
            let at = latest_at(index, store, &id, "content")?.map_or(0, |(t, _)| t);
            if kind == "decision" {
                let status =
                    latest(index, store, &id, "status")?.unwrap_or_else(|| "recorded".to_string());
                decisions.push((at, slug, title, status));
                decision_sids.insert(id);
            } else {
                plans.push((at, slug, title));
            }
        }
    }
    decisions.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    ins.decisions = decisions
        .into_iter()
        .map(|(_, s, t, st)| (s, t, st))
        .collect();
    plans.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    ins.plans = plans.into_iter().map(|(_, s, t)| (s, t)).collect();

    // Skills and agent configuration under this prefix.
    for kind in ["skill", "agent_config"] {
        let mut seen: BTreeSet<StableId> = BTreeSet::new();
        let mut rows: Vec<(String, String, String)> = Vec::new();
        for node in index.entities_by_kind(kind) {
            let Ok(Object::Entity { id, labels, .. }) = store.get(&node) else {
                continue;
            };
            if labels.get("prefix").map(String::as_str) != Some(prefix) || !seen.insert(id.clone())
            {
                continue;
            }
            if !crate::lifecycle::of(index, store, &id)?.0.is_active() {
                continue;
            }
            let slug = labels.get("slug").cloned().unwrap_or_default();
            let agent = latest(index, store, &id, "agent")?
                .or_else(|| labels.get("agent").cloned())
                .unwrap_or_else(|| "generic".to_string());
            let third = if kind == "skill" {
                latest(index, store, &id, "description")?
                    .or_else(|| latest(index, store, &id, "name").ok().flatten())
                    .unwrap_or_else(|| slug.clone())
            } else {
                latest(index, store, &id, "role")?.unwrap_or_else(|| "config".to_string())
            };
            rows.push((slug, agent, third));
        }
        rows.sort();
        if kind == "skill" {
            ins.skills = rows;
        } else {
            ins.agent_configs = rows;
        }
    }

    // Documents failing their template contract (recorded, never enforced),
    // and documents gone stale: a mentioned file changed after the doc did.
    // Kinds = the built-in families plus every graph-defined capture kind.
    let builtin_kinds = ["decision", "plan", "skill", "agent_config"];
    let kind_registry = crate::kinds::registry(store, index)?;
    let doc_kinds = crate::kinds::doc_kinds(store, index)?;
    for kind in &doc_kinds {
        let kind = kind.as_str();
        let rot = rot_severity(
            kind_registry
                .get(kind)
                .map(|d| d.rot.as_str())
                .unwrap_or(""),
            kind,
        );
        let mut count = 0usize;
        let mut seen: BTreeSet<StableId> = BTreeSet::new();
        for node in index.entities_by_kind(kind) {
            let Ok(Object::Entity { id, labels, .. }) = store.get(&node) else {
                continue;
            };
            if labels.get("prefix").map(String::as_str) != Some(prefix) || !seen.insert(id.clone())
            {
                continue;
            }
            if !crate::lifecycle::of(index, store, &id)?.0.is_active() {
                continue; // history neither rots nor violates contracts
            }
            let slug = labels.get("slug").cloned().unwrap_or_default();
            count += 1;
            if latest(index, store, &id, "conforms")?.as_deref() == Some("false") {
                let missing = latest(index, store, &id, "missing")?.unwrap_or_default();
                ins.nonconforming
                    .push((slug.clone(), kind.to_string(), missing));
            }
            let Some(severity) = rot else { continue };
            if let Some((doc_at, _)) = latest_at(index, store, &id, "content")? {
                // Acknowledgement resets the clock: "reviewed against the
                // present" counts as freshly written, file untouched.
                let effective = latest_at(index, store, &id, "reviewed")?
                    .map_or(doc_at, |(ack_at, _)| doc_at.max(ack_at));
                let mut changed = Vec::new();
                for (_, to) in live_from(index, store, &id, "mentions")? {
                    if let Some((f_at, _)) = latest_at(index, store, &to, "content_b3")? {
                        if f_at > effective {
                            changed.push(sid_label(index, store, &to));
                        }
                    }
                }
                if !changed.is_empty() {
                    changed.sort();
                    ins.stale_docs.push(StaleDoc {
                        slug,
                        kind: kind.to_string(),
                        severity,
                        changed,
                    });
                }
            }
        }
        if !builtin_kinds.contains(&kind) && count > 0 {
            ins.custom_artifacts.push((kind.to_string(), count));
        }
    }
    ins.nonconforming.sort();
    // Assets rot too: a declared `depicts` target that changed after the
    // asset's bytes were captured. Same shape, same surfaces.
    let asset_rot = rot_severity(
        kind_registry
            .get("asset")
            .map(|d| d.rot.as_str())
            .unwrap_or(""),
        "asset",
    );
    if let Some(severity) = asset_rot {
        for (slug, changed) in crate::assets::stale(store, index, prefix)? {
            ins.stale_docs.push(StaleDoc {
                slug,
                kind: "asset".to_string(),
                severity,
                changed,
            });
        }
    }
    ins.stale_docs.sort_by(|a, b| {
        a.severity
            .cmp(&b.severity)
            .then_with(|| a.slug.cmp(&b.slug))
    });

    // Features: done-ness evaluated live against the definition of done.
    for row in crate::features::list(store, index, prefix)? {
        let report = crate::features::evaluate(store, index, prefix, &row.slug)?;
        let (met, total) = report.score();
        ins.features.push(FeatureProgress {
            slug: row.slug,
            status: row.status,
            fraction: format!("{met}/{total}"),
            done: report.done,
            by_parts: report.by_parts(),
        });
    }

    let mut churn: Vec<(String, usize)> = Vec::new();
    let mut hubs: Vec<(String, usize)> = Vec::new();
    let mut largest: Vec<(String, usize)> = Vec::new();
    let mut untested: Vec<(String, usize)> = Vec::new();
    let mut modules: BTreeMap<String, usize> = BTreeMap::new();

    for (rel, sid) in &file_sids {
        if latest(index, store, sid, "present")?.as_deref() == Some("false") {
            ins.deleted_files += 1;
            continue;
        }
        ins.files += 1;

        let versions = index
            .observations_of(sid)
            .iter()
            .filter_map(|id| store.get(id).ok())
            .filter(
                |o| matches!(o, Object::Observation { property, .. } if property == "content_b3"),
            )
            .count();
        // Generated projections churn by design; their edits are noise.
        let generated = latest(index, store, sid, "generated")?.as_deref() == Some("true");
        if versions > 1 && !generated {
            churn.push((rel.clone(), versions));
        }

        let contains = live_from(index, store, sid, "contains")?.len();
        ins.relations += contains;
        if contains > 0 {
            largest.push((rel.clone(), contains));
        }
        ins.symbols += contains;

        let declared: usize = latest(index, store, sid, "tests_declared")?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        ins.tests_declared += declared;
        if latest(index, store, sid, "file_role")?.as_deref() == Some("test") {
            ins.test_files += 1;
        }

        let importers = live_to(index, store, sid, "imports")?.len();
        if importers > 0 {
            hubs.push((rel.clone(), importers));
            // A hub nobody tests is concentrated risk: no declared tests
            // in the file, no test file covering it.
            if declared == 0 && live_to(index, store, sid, "covers")?.is_empty() {
                untested.push((rel.clone(), importers));
            }
        }

        // Is this file covered by a decision? (Any `mentions` from an ADR.)
        for (_, from) in live_to(index, store, sid, "mentions")? {
            if decision_sids.contains(&from) {
                ins.decided.insert(rel.clone());
                break;
            }
        }

        for (_, to) in live_from(index, store, sid, "imports")? {
            ins.relations += 1;
            for node in index.entity_nodes(&to) {
                if let Ok(Object::Entity {
                    entity_kind,
                    labels,
                    ..
                }) = store.get(&node)
                {
                    if entity_kind == "module" {
                        let name = labels.get("name").cloned().unwrap_or_default();
                        *modules.entry(name).or_default() += 1;
                    }
                    break;
                }
            }
        }
    }

    // Full lists, sorted strongest-first; rendering truncates honestly
    // ("showing 5 of 12") — a truncated list must never pose as a total.
    let ranked = |mut v: Vec<(String, usize)>| {
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        v
    };
    ins.churn = ranked(churn);
    ins.hubs = ranked(hubs);
    ins.largest = ranked(largest);
    ins.untested_hubs = ranked(untested);
    ins.external_modules = ranked(modules.into_iter().collect());

    // Test protocols: the latest imported run and currently-failing cases.
    if let Some((at, total, passed, failed, _)) =
        testing::runs(store, index, prefix)?.into_iter().next()
    {
        ins.last_run = Some((at, total, passed, failed));
    }
    ins.failing = testing::failing_cases(store, index, prefix)?;

    // Notes across all twinned files plus the repo entity, newest first.
    let repo_sid = StableId::derive(&["repo", prefix]);
    let mut note_subjects = file_sids.clone();
    note_subjects.push((prefix.to_string(), repo_sid.clone()));
    let mut all_notes: Vec<(u64, String, String)> = Vec::new();
    for (rel, sid) in &note_subjects {
        for (at, text) in notes(index, store, sid)? {
            all_notes.push((at, rel.clone(), text));
        }
    }
    all_notes.sort_by(|a, b| b.0.cmp(&a.0));
    ins.notes = all_notes;

    ins.git_commit = latest(index, store, &repo_sid, "git_commit")?;
    ins.git_branch = latest(index, store, &repo_sid, "git_branch")?;

    // Growth and quality series: pair up the repo entity's observations
    // by time, one pass for both. A quality point is kept by presence,
    // not by being non-zero — an all-zero first reading is a real point.
    #[derive(Default)]
    struct QRaw {
        tests_passed: usize,
        tests_total: usize,
        features_done: usize,
        features_total: usize,
        stale: usize,
        uncorr: usize,
        seen: bool,
    }
    let mut points: BTreeMap<u64, (usize, usize, usize)> = BTreeMap::new();
    let mut qpoints: BTreeMap<u64, QRaw> = BTreeMap::new();
    for id in index.observations_of(&repo_sid) {
        if let Object::Observation {
            property,
            value,
            observed_at_ms,
            ..
        } = store.get(&id)?
        {
            let Ok(n) = value.parse::<usize>() else {
                continue;
            };
            let mut quality = |set: fn(&mut QRaw, usize)| {
                let q = qpoints.entry(observed_at_ms).or_default();
                set(q, n);
                q.seen = true;
            };
            match property.as_str() {
                "files_present" => points.entry(observed_at_ms).or_insert((0, 0, 0)).0 = n,
                "symbols_total" => points.entry(observed_at_ms).or_insert((0, 0, 0)).1 = n,
                "relations_total" => points.entry(observed_at_ms).or_insert((0, 0, 0)).2 = n,
                "tests_passed" => quality(|q, n| q.tests_passed = n),
                "tests_total" => quality(|q, n| q.tests_total = n),
                "features_done" => quality(|q, n| q.features_done = n),
                "features_total" => quality(|q, n| q.features_total = n),
                "stale_warn_total" => quality(|q, n| q.stale = n),
                "uncorroborated_total" => quality(|q, n| q.uncorr = n),
                _ => {}
            }
        }
    }
    ins.series = points
        .into_iter()
        .filter(|(_, (f, s, r))| *f + *s + *r > 0)
        .map(|(at, (f, s, r))| (at, f, s, r))
        .collect();
    ins.quality = qpoints
        .into_iter()
        .filter(|(_, q)| q.seen)
        .map(|(at, q)| QualityPoint {
            at_ms: at,
            tests: (q.tests_total > 0).then_some((q.tests_passed, q.tests_total)),
            features_done: q.features_done,
            features_total: q.features_total,
            stale_warnings: q.stale,
            uncorroborated: q.uncorr,
        })
        .collect();

    Ok(ins)
}
