//! The feature registry: features as entities, done-ness as a query.
//!
//! A feature is an explicit declaration (agents or humans register it),
//! linked into the graph by relations: `implemented_by` source files,
//! `tested_by` test files, `decided_by` ADRs, `documented_in` docs. The
//! definition of done is the `feature` template's `requires` list — so
//! "is it done?" is evaluated against graph state, never against a vibe,
//! and the feature matrix is a rendered query, not a spreadsheet that rots.

use crate::templates;
use crate::twin::{latest, observe_src, relate};
use brain_core::ids::StableId;
use brain_core::object::Object;
use brain_index::{replay, Index, MemIndex};
use brain_store::{now_ms, Store, StoreError};
use std::collections::{BTreeMap, BTreeSet};

/// The fallback definition of done when no `feature` template is seeded.
pub const DEFAULT_DOD: &[&str] = &["implemented_by", "tested_by", "decided_by", "documented_in"];

pub fn feature_sid(prefix: &str, slug: &str) -> StableId {
    StableId::derive(&["feature", prefix, slug])
}

/// Register (or update — every write is guarded) a feature under a prefix.
pub fn add(
    store: &Store,
    prefix: &str,
    slug: &str,
    title: &str,
    status: &str,
) -> Result<(StableId, bool), StoreError> {
    let mut index = MemIndex::new();
    replay(store, &mut index)?;
    let now = now_ms();
    let sid = feature_sid(prefix, slug);
    let mut labels = BTreeMap::new();
    labels.insert("prefix".to_string(), prefix.to_string());
    labels.insert("slug".to_string(), slug.to_string());
    labels.insert("title".to_string(), title.to_string());
    store.put(&Object::Entity {
        id: sid.clone(),
        entity_kind: "feature".to_string(),
        labels,
    })?;
    let mut wrote = false;
    for (prop, value) in [("title", title), ("status", status)] {
        if latest(&index, store, &sid, prop)?.as_deref() != Some(value) {
            observe_src(store, &sid, prop, value, "agent", now)?;
            wrote = true;
        }
    }
    let repo_sid = StableId::derive(&["repo", prefix]);
    let mut written = BTreeSet::new();
    if relate(store, &index, &mut written, &sid, "concerns", &repo_sid, now)? {
        wrote = true;
    }
    Ok((sid, wrote))
}

/// Resolve a link target name to an existing entity: a twinned file path,
/// the slug of any registered artifact kind (built-in or taught), or a
/// change/test entity. Returns the entity's stable id and its kind.
/// Built-in kinds are tried first so historical resolution order holds.
pub fn resolve_target(
    store: &Store,
    index: &MemIndex,
    prefix: &str,
    name: &str,
) -> Result<Option<(StableId, String)>, StoreError> {
    let mut kinds: Vec<String> =
        ["decision", "plan", "skill", "agent_config", "feature"]
            .iter()
            .map(|s| s.to_string())
            .collect();
    for kind in crate::kinds::registry(store, index)?.keys() {
        if !kinds.contains(kind) {
            kinds.push(kind.clone());
        }
    }
    kinds.extend(["change", "test_run", "test_case"].iter().map(|s| s.to_string()));

    let file = StableId::derive(&["file", name]);
    if !index.entity_nodes(&file).is_empty() {
        return Ok(Some((file, "file".to_string())));
    }
    for kind in kinds {
        // test_case entities derive under "test", not their entity kind.
        let derive_kind = if kind == "test_case" { "test" } else { &kind };
        let sid = StableId::derive(&[derive_kind, prefix, name]);
        if !index.entity_nodes(&sid).is_empty() {
            return Ok(Some((sid, kind)));
        }
    }
    Ok(None)
}

/// Link a feature to a target entity by predicate. Guarded: an existing
/// identical relation writes nothing.
pub fn link(
    store: &Store,
    prefix: &str,
    slug: &str,
    predicate: &str,
    target: &StableId,
) -> Result<bool, StoreError> {
    let mut index = MemIndex::new();
    replay(store, &mut index)?;
    let mut written = BTreeSet::new();
    relate(
        store,
        &index,
        &mut written,
        &feature_sid(prefix, slug),
        predicate,
        target,
        now_ms(),
    )
}

#[derive(Debug)]
pub struct DoneCheck {
    pub predicate: String,
    /// Distinct linked targets satisfying the predicate.
    pub count: usize,
}

#[derive(Debug)]
pub struct DoneReport {
    pub checks: Vec<DoneCheck>,
    pub done: bool,
}

/// The definition of done: the `feature` template's `requires` list from
/// the graph, or the built-in default when none is seeded.
pub fn dod(store: &Store, index: &MemIndex) -> Result<Vec<String>, StoreError> {
    Ok(templates::by_kind(store, index)?
        .get("feature")
        .map(|(_, r)| r.clone())
        .unwrap_or_else(|| DEFAULT_DOD.iter().map(|s| s.to_string()).collect()))
}

/// Evaluate a feature against the definition of done — pure graph state.
pub fn evaluate(
    store: &Store,
    index: &MemIndex,
    prefix: &str,
    slug: &str,
) -> Result<DoneReport, StoreError> {
    let sid = feature_sid(prefix, slug);
    let mut checks = Vec::new();
    for predicate in dod(store, index)? {
        let mut targets: BTreeSet<StableId> = BTreeSet::new();
        for (_, to) in crate::twin::live_from(index, store, &sid, &predicate)? {
            targets.insert(to);
        }
        checks.push(DoneCheck { predicate, count: targets.len() });
    }
    let done = !checks.is_empty() && checks.iter().all(|c| c.count > 0);
    Ok(DoneReport { checks, done })
}

/// Record the evaluation as a guarded observation on the feature: `done`
/// flips are timeline events, not overwrites.
pub fn record_done(
    store: &Store,
    index: &MemIndex,
    prefix: &str,
    slug: &str,
    report: &DoneReport,
) -> Result<bool, StoreError> {
    let sid = feature_sid(prefix, slug);
    let value = if report.done { "true" } else { "false" };
    if latest(index, store, &sid, "done")?.as_deref() != Some(value) {
        observe_src(store, &sid, "done", value, "dod", now_ms())?;
        return Ok(true);
    }
    Ok(false)
}

#[derive(Debug)]
pub struct FeatureRow {
    pub slug: String,
    pub title: String,
    pub status: String,
    /// Last recorded `done` observation, if any.
    pub done: Option<String>,
}

/// All features under a prefix, sorted by slug.
pub fn list(
    store: &Store,
    index: &MemIndex,
    prefix: &str,
) -> Result<Vec<FeatureRow>, StoreError> {
    let mut seen: BTreeSet<StableId> = BTreeSet::new();
    let mut out = Vec::new();
    for node in index.entities_by_kind("feature") {
        let Ok(Object::Entity { id, labels, .. }) = store.get(&node) else { continue };
        if labels.get("prefix").map(String::as_str) != Some(prefix) || !seen.insert(id.clone()) {
            continue;
        }
        let slug = labels.get("slug").cloned().unwrap_or_default();
        let title = latest(index, store, &id, "title")?
            .or_else(|| labels.get("title").cloned())
            .unwrap_or_else(|| slug.clone());
        let status =
            latest(index, store, &id, "status")?.unwrap_or_else(|| "planned".to_string());
        let done = latest(index, store, &id, "done")?;
        out.push(FeatureRow { slug, title, status, done });
    }
    out.sort_by(|a, b| a.slug.cmp(&b.slug));
    Ok(out)
}
