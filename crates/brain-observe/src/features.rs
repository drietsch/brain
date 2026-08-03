//! The feature registry: features as entities, done-ness as a query.
//!
//! A feature is an explicit declaration (agents or humans register it),
//! linked into the graph by relations: `implemented_by` source files,
//! `tested_by` test files, `decided_by` ADRs, `documented_in` docs. The
//! definition of done is the `feature` template's `requires` list — so
//! "is it done?" is evaluated against graph state, never against a vibe,
//! and the feature matrix is a rendered query, not a spreadsheet that rots.
//!
//! **Features have parts.** A real feature is rarely one claim: it has a
//! core, an API, a user interface, tests, documentation — and each part is
//! separately buildable, testable and provable. Parts are themselves
//! features, joined by `part_of` (child → parent), so a part has its own
//! definition of done, its own evidence and its own page. Depth is not
//! limited to two.
//!
//! The rollup rule is the one that matters: **a feature with parts is
//! judged by its parts.** Its own links still show as evidence, but they
//! cannot make it ready while a part is not — otherwise a parent could be
//! declared done by attaching four files to it and ignoring the work.
//! A feature without parts is judged by its own definition of done, which
//! is exactly the old behaviour.

use crate::templates;
use crate::twin::{latest, latest_at_before, observe_src, relate};
use brain_core::ids::StableId;
use brain_core::object::Object;
use brain_index::{replay, Index, MemIndex};
use brain_store::{now_ms, Store, StoreError};
use std::collections::{BTreeMap, BTreeSet};

/// The fallback definition of done when no `feature` template is seeded.
pub const DEFAULT_DOD: &[&str] = &["implemented_by", "tested_by", "decided_by", "documented_in"];

/// The composition predicate, written child → parent.
///
/// That direction keeps the append-only store honest: adding a part never
/// rewrites the parent's edges, and `brain feature link p core part_of
/// authentication` reads the way it means.
pub const PART_OF: &str = "part_of";

/// How deep composition may nest before we stop walking.
///
/// Nothing traversed features before this, so nothing protected against a
/// cycle. The visited set below makes cycles safe; this makes them cheap.
pub const MAX_DEPTH: usize = 12;

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
    if relate(
        store,
        &index,
        &mut written,
        &sid,
        "concerns",
        &repo_sid,
        now,
    )? {
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
    resolve_target_as(store, index, prefix, name, None)
}

/// `resolve_target`, optionally pinned to one kind.
///
/// The unpinned order tries `decision`, `plan`, `skill` and `agent_config`
/// before `feature`, so a part named `eyes` would silently resolve to an
/// ADR that happens to share the slug. A composition edge has to land on a
/// feature, so callers that know the kind say so.
pub fn resolve_target_as(
    store: &Store,
    index: &MemIndex,
    prefix: &str,
    name: &str,
    want: Option<&str>,
) -> Result<Option<(StableId, String)>, StoreError> {
    if let Some(kind) = want {
        if kind == "file" {
            let file = StableId::derive(&["file", name]);
            return Ok((!index.entity_nodes(&file).is_empty())
                .then(|| (file, "file".to_string())));
        }
        let derive_kind = if kind == "test_case" { "test" } else { kind };
        let sid = StableId::derive(&[derive_kind, prefix, name]);
        return Ok((!index.entity_nodes(&sid).is_empty())
            .then(|| (sid, kind.to_string())));
    }

    let mut kinds: Vec<String> = ["decision", "plan", "skill", "agent_config", "feature"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    for kind in crate::kinds::registry(store, index)?.keys() {
        if !kinds.contains(kind) {
            kinds.push(kind.clone());
        }
    }
    kinds.extend(
        ["change", "test_run", "test_case"]
            .iter()
            .map(|s| s.to_string()),
    );

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
    let from = feature_sid(prefix, slug);

    // Composition is the one predicate with structural rules. Everything
    // else stays a free assertion, as it has always been.
    if predicate == PART_OF {
        if *target == from {
            return Err(StoreError::Io(std::io::Error::other(
                "a feature cannot be part of itself",
            )));
        }
        if slug_of(store, &index, target).is_empty() {
            return Err(StoreError::Io(std::io::Error::other(
                "a feature can only be part of another feature",
            )));
        }
        // Walking up from the intended parent must not reach this feature.
        if ancestry(store, &index, target)?
            .iter()
            .any(|(sid, _)| *sid == from)
        {
            return Err(StoreError::Io(std::io::Error::other(
                "that would make a loop: the parent is already part of this feature",
            )));
        }
        if let Some((_, existing)) = parent(store, &index, &from)? {
            let wanted = slug_of(store, &index, target);
            if existing != wanted {
                return Err(StoreError::Io(std::io::Error::other(format!(
                    "'{slug}' is already part of '{existing}' — retract that first: \
                     brain relation retract {slug} part_of {existing} --prefix {prefix}"
                ))));
            }
        }
    }

    let mut written = BTreeSet::new();
    relate(
        store,
        &index,
        &mut written,
        &from,
        predicate,
        target,
        now_ms(),
    )
}

#[derive(Debug, serde::Serialize)]
pub struct DoneCheck {
    pub predicate: String,
    /// Distinct linked targets satisfying the predicate.
    pub count: usize,
}

/// One part of a feature, already evaluated.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PartReport {
    pub slug: String,
    pub title: String,
    pub done: bool,
    /// How many of its own requirements (or parts) are satisfied.
    pub met: usize,
    pub total: usize,
    /// Its own parts, if it has any.
    pub parts: Vec<PartReport>,
}

#[derive(Debug, serde::Serialize)]
pub struct DoneReport {
    pub checks: Vec<DoneCheck>,
    pub done: bool,
    /// Parts of this feature, evaluated. Empty for a leaf.
    pub parts: Vec<PartReport>,
    /// When parts exist and one of them is not ready, the first such part
    /// — so a parent can say *what* is holding it up rather than only
    /// that something is.
    pub blocked_by: Option<String>,
}

impl DoneReport {
    /// Whether this feature is judged by its parts rather than its own
    /// definition of done.
    pub fn by_parts(&self) -> bool {
        !self.parts.is_empty()
    }

    /// Satisfied over total, in whichever terms this feature is judged.
    pub fn score(&self) -> (usize, usize) {
        if self.by_parts() {
            (
                self.parts.iter().filter(|p| p.done).count(),
                self.parts.len(),
            )
        } else {
            (
                self.checks.iter().filter(|c| c.count > 0).count(),
                self.checks.len(),
            )
        }
    }
}

/// The definition of done: the `feature` template's `requires` list from
/// the graph, or the built-in default when none is seeded.
pub fn dod(store: &Store, index: &MemIndex) -> Result<Vec<String>, StoreError> {
    Ok(templates::by_kind(store, index)?
        .get("feature")
        .map(|(_, r)| r.clone())
        .unwrap_or_else(|| DEFAULT_DOD.iter().map(|s| s.to_string()).collect()))
}

/// The parts of a feature, in registration order by slug.
///
/// Parts point at their parent, so the children of a parent are the
/// incoming `part_of` edges.
pub fn children(
    store: &Store,
    index: &MemIndex,
    sid: &StableId,
) -> Result<Vec<(StableId, String)>, StoreError> {
    children_at(store, index, sid, u64::MAX)
}

/// The parts of a feature as they stood at `t`.
pub fn children_at(
    store: &Store,
    index: &MemIndex,
    sid: &StableId,
    t: u64,
) -> Result<Vec<(StableId, String)>, StoreError> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for (_, from) in crate::twin::live_to_at(index, store, sid, PART_OF, t)? {
        if !seen.insert(from.clone()) {
            continue;
        }
        let slug = slug_of(store, index, &from);
        if !slug.is_empty() {
            out.push((from, slug));
        }
    }
    out.sort_by(|a, b| a.1.cmp(&b.1));
    Ok(out)
}

/// The feature this one is part of, if any.
pub fn parent(
    store: &Store,
    index: &MemIndex,
    sid: &StableId,
) -> Result<Option<(StableId, String)>, StoreError> {
    for (_, to) in crate::twin::live_from(index, store, sid, PART_OF)? {
        let slug = slug_of(store, index, &to);
        if !slug.is_empty() {
            return Ok(Some((to, slug)));
        }
    }
    Ok(None)
}

/// The chain from a feature up to its outermost ancestor, nearest first.
pub fn ancestry(
    store: &Store,
    index: &MemIndex,
    sid: &StableId,
) -> Result<Vec<(StableId, String)>, StoreError> {
    let mut out = Vec::new();
    let mut seen: BTreeSet<StableId> = BTreeSet::from([sid.clone()]);
    let mut current = sid.clone();
    while out.len() < MAX_DEPTH {
        let Some((up, slug)) = parent(store, index, &current)? else {
            break;
        };
        if !seen.insert(up.clone()) {
            break; // a cycle; stop rather than spin
        }
        out.push((up.clone(), slug));
        current = up;
    }
    Ok(out)
}

fn slug_of(store: &Store, index: &MemIndex, sid: &StableId) -> String {
    for node in index.entity_nodes(sid) {
        if let Ok(Object::Entity {
            entity_kind,
            labels,
            ..
        }) = store.get(&node)
        {
            if entity_kind == "feature" {
                return labels.get("slug").cloned().unwrap_or_default();
            }
        }
    }
    String::new()
}

/// Evaluate a feature against the definition of done — pure graph state.
///
/// A feature with parts is judged by its parts; a feature without parts is
/// judged by its own requirements.
pub fn evaluate(
    store: &Store,
    index: &MemIndex,
    prefix: &str,
    slug: &str,
) -> Result<DoneReport, StoreError> {
    evaluate_at(store, index, prefix, slug, u64::MAX)
}

/// The feature judged as it stood at `t`: links and parts that existed
/// then, counted then. The definition of done is read from the present
/// template — the past is judged by today's bar, and any surface that
/// renders a past judgment says so.
pub fn evaluate_at(
    store: &Store,
    index: &MemIndex,
    prefix: &str,
    slug: &str,
    t: u64,
) -> Result<DoneReport, StoreError> {
    let requires = dod(store, index)?;
    let mut seen = BTreeSet::new();
    evaluate_with(store, index, prefix, slug, &requires, &mut seen, 0, t)
}

#[allow(clippy::too_many_arguments)]
fn evaluate_with(
    store: &Store,
    index: &MemIndex,
    prefix: &str,
    slug: &str,
    requires: &[String],
    seen: &mut BTreeSet<StableId>,
    depth: usize,
    t: u64,
) -> Result<DoneReport, StoreError> {
    let sid = feature_sid(prefix, slug);
    let mut checks = Vec::new();
    for predicate in requires {
        let mut targets: BTreeSet<StableId> = BTreeSet::new();
        for (_, to) in crate::twin::live_from_at(index, store, &sid, predicate, t)? {
            targets.insert(to);
        }
        checks.push(DoneCheck {
            predicate: predicate.clone(),
            count: targets.len(),
        });
    }

    // A feature already on the path is a cycle: evaluate it as a leaf
    // rather than following the loop.
    let mut parts = Vec::new();
    if depth < MAX_DEPTH && seen.insert(sid.clone()) {
        for (child, child_slug) in children_at(store, index, &sid, t)? {
            let _ = child;
            let report =
                evaluate_with(store, index, prefix, &child_slug, requires, seen, depth + 1, t)?;
            let (met, total) = report.score();
            parts.push(PartReport {
                title: title_of(store, index, prefix, &child_slug),
                slug: child_slug,
                done: report.done,
                met,
                total,
                parts: report.parts,
            });
        }
    }

    let done = if parts.is_empty() {
        !checks.is_empty() && checks.iter().all(|c| c.count > 0)
    } else {
        parts.iter().all(|p| p.done)
    };
    let blocked_by = parts
        .iter()
        .find(|p| !p.done)
        .map(|p| p.title.clone());

    Ok(DoneReport {
        checks,
        done,
        parts,
        blocked_by,
    })
}

fn title_of(store: &Store, index: &MemIndex, prefix: &str, slug: &str) -> String {
    let sid = feature_sid(prefix, slug);
    latest(index, store, &sid, "title")
        .ok()
        .flatten()
        .unwrap_or_else(|| slug.to_string())
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
pub fn list(store: &Store, index: &MemIndex, prefix: &str) -> Result<Vec<FeatureRow>, StoreError> {
    let mut seen: BTreeSet<StableId> = BTreeSet::new();
    let mut out = Vec::new();
    for node in index.entities_by_kind("feature") {
        let Ok(Object::Entity { id, labels, .. }) = store.get(&node) else {
            continue;
        };
        if labels.get("prefix").map(String::as_str) != Some(prefix) || !seen.insert(id.clone()) {
            continue;
        }
        let slug = labels.get("slug").cloned().unwrap_or_default();
        let title = latest(index, store, &id, "title")?
            .or_else(|| labels.get("title").cloned())
            .unwrap_or_else(|| slug.clone());
        let status = latest(index, store, &id, "status")?.unwrap_or_else(|| "planned".to_string());
        let done = latest(index, store, &id, "done")?;
        out.push(FeatureRow {
            slug,
            title,
            status,
            done,
        });
    }
    out.sort_by(|a, b| a.slug.cmp(&b.slug));
    Ok(out)
}

/// Features as they existed at `t`. An Entity object carries no time of
/// its own, so the first guarded observation — a title or a status — is
/// the honest birth certificate: a feature registered after `t` has
/// neither and does not appear.
pub fn list_at(
    store: &Store,
    index: &MemIndex,
    prefix: &str,
    t: u64,
) -> Result<Vec<FeatureRow>, StoreError> {
    let mut seen: BTreeSet<StableId> = BTreeSet::new();
    let mut out = Vec::new();
    for node in index.entities_by_kind("feature") {
        let Ok(Object::Entity { id, labels, .. }) = store.get(&node) else {
            continue;
        };
        if labels.get("prefix").map(String::as_str) != Some(prefix) || !seen.insert(id.clone()) {
            continue;
        }
        let title_at = latest_at_before(index, store, &id, "title", t)?;
        let status_at = latest_at_before(index, store, &id, "status", t)?;
        if title_at.is_none() && status_at.is_none() {
            continue;
        }
        let slug = labels.get("slug").cloned().unwrap_or_default();
        let title = title_at
            .map(|(_, v)| v)
            .or_else(|| labels.get("title").cloned())
            .unwrap_or_else(|| slug.clone());
        let status = status_at
            .map(|(_, v)| v)
            .unwrap_or_else(|| "planned".to_string());
        let done = latest_at_before(index, store, &id, "done", t)?.map(|(_, v)| v);
        out.push(FeatureRow {
            slug,
            title,
            status,
            done,
        });
    }
    out.sort_by(|a, b| a.slug.cmp(&b.slug));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_index(store: &Store) -> MemIndex {
        let mut index = MemIndex::new();
        replay(store, &mut index).unwrap();
        index
    }

    /// A workspace with one file, so a definition-of-done slot can be
    /// satisfied for real rather than by a fixture shortcut.
    fn world() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "pub fn thing() {}\n").unwrap();
        std::fs::create_dir_all(dir.path().join("docs/adr")).unwrap();
        std::fs::write(
            dir.path().join("docs/adr/adr-001-shape.md"),
            "# Shape\n\nStatus: accepted\n\nAbout src/lib.rs.\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("docs/guide.md"), "# Guide\n\nAbout src/lib.rs.\n").unwrap();
        let store = Store::open(&dir.path().join(".brain")).unwrap();
        crate::templates::seed(&store).unwrap();
        crate::twin::refresh(&store, dir.path(), "twin/app").unwrap();
        (dir, store)
    }

    /// Satisfy every requirement of a leaf feature.
    fn complete(store: &Store, slug: &str) {
        let index = fresh_index(store);
        let file = StableId::derive(&["file", "src/lib.rs"]);
        let decision = StableId::derive(&["decision", "twin/app", "adr-001-shape"]);
        let doc = StableId::derive(&["file", "docs/guide.md"]);
        let _ = index;
        for (predicate, target) in [
            ("implemented_by", &file),
            ("tested_by", &file),
            ("decided_by", &decision),
            ("documented_in", &doc),
        ] {
            link(store, "twin/app", slug, predicate, target).unwrap();
        }
    }

    /// A feature is judged by the links that existed at the moment asked
    /// about: a link added later does not reach back, and a link
    /// retracted later still counts at a moment before the retraction.
    #[test]
    fn a_feature_is_judged_at_the_moment_you_ask() {
        let (_dir, store) = world();
        add(&store, "twin/app", "aged", "Aged", "building").unwrap();
        let file = StableId::derive(&["file", "src/lib.rs"]);
        link(&store, "twin/app", "aged", "implemented_by", &file).unwrap();

        std::thread::sleep(std::time::Duration::from_millis(5));
        let t = now_ms();
        std::thread::sleep(std::time::Duration::from_millis(5));
        link(&store, "twin/app", "aged", "tested_by", &file).unwrap();

        let index = fresh_index(&store);
        let count_of = |r: &DoneReport, p: &str| {
            r.checks
                .iter()
                .find(|c| c.predicate == p)
                .map(|c| c.count)
                .unwrap_or(0)
        };
        let now_report = evaluate(&store, &index, "twin/app", "aged").unwrap();
        let then_report = evaluate_at(&store, &index, "twin/app", "aged", t).unwrap();
        assert_eq!(count_of(&now_report, "tested_by"), 1);
        assert_eq!(
            count_of(&then_report, "tested_by"),
            0,
            "a later link does not reach back"
        );
        assert_eq!(count_of(&then_report, "implemented_by"), 1);

        // Retraction is equally time-honest: gone now, held then.
        std::thread::sleep(std::time::Duration::from_millis(5));
        let before_retraction = now_ms();
        std::thread::sleep(std::time::Duration::from_millis(5));
        crate::twin::retract(&store, &feature_sid("twin/app", "aged"), "implemented_by", &file)
            .unwrap();
        let index = fresh_index(&store);
        let now_report = evaluate(&store, &index, "twin/app", "aged").unwrap();
        let then_report =
            evaluate_at(&store, &index, "twin/app", "aged", before_retraction).unwrap();
        assert_eq!(count_of(&now_report, "implemented_by"), 0);
        assert_eq!(
            count_of(&then_report, "implemented_by"),
            1,
            "the past keeps its link"
        );
    }

    /// Entities carry no time; the first guarded observation is the
    /// birth certificate list_at reads.
    #[test]
    fn a_feature_registered_later_did_not_exist_then() {
        let (_dir, store) = world();
        add(&store, "twin/app", "first", "First", "building").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let t = now_ms();
        std::thread::sleep(std::time::Duration::from_millis(5));
        add(&store, "twin/app", "second", "Second", "building").unwrap();

        let index = fresh_index(&store);
        let then = list_at(&store, &index, "twin/app", t).unwrap();
        assert!(then.iter().any(|f| f.slug == "first"));
        assert!(
            !then.iter().any(|f| f.slug == "second"),
            "not yet registered at that moment"
        );
        let present = list_at(&store, &index, "twin/app", u64::MAX).unwrap();
        assert!(present.iter().any(|f| f.slug == "second"));
    }

    #[test]
    fn a_feature_without_parts_is_judged_exactly_as_before() {
        let (_dir, store) = world();
        add(&store, "twin/app", "solo", "Solo", "building").unwrap();
        let index = fresh_index(&store);
        let report = evaluate(&store, &index, "twin/app", "solo").unwrap();
        assert!(!report.done);
        assert!(!report.by_parts());
        assert_eq!(report.score(), (0, 4));

        complete(&store, "solo");
        let index = fresh_index(&store);
        let report = evaluate(&store, &index, "twin/app", "solo").unwrap();
        assert!(report.done);
        assert_eq!(report.score(), (4, 4));
        assert!(report.parts.is_empty());
        assert_eq!(report.blocked_by, None);
    }

    #[test]
    fn a_feature_with_parts_is_judged_by_its_parts() {
        let (_dir, store) = world();
        add(&store, "twin/app", "auth", "Authentication", "building").unwrap();
        for (slug, title) in [("auth-core", "Core"), ("auth-ux", "UX/UI")] {
            add(&store, "twin/app", slug, title, "building").unwrap();
            let parent = feature_sid("twin/app", "auth");
            link(&store, "twin/app", slug, PART_OF, &parent).unwrap();
        }

        // The parent's own requirements are fully satisfied — and that is
        // deliberately not enough while a part is unfinished. Otherwise a
        // parent could be declared done by attaching four files to it.
        complete(&store, "auth");
        complete(&store, "auth-core");

        let index = fresh_index(&store);
        let report = evaluate(&store, &index, "twin/app", "auth").unwrap();
        assert!(report.by_parts());
        assert!(
            report.checks.iter().all(|c| c.count > 0),
            "the parent's own links are all satisfied"
        );
        assert!(!report.done, "but a part is not ready, so neither is it");
        assert_eq!(report.score(), (1, 2));
        assert_eq!(
            report.blocked_by.as_deref(),
            Some("UX/UI"),
            "and it names what is holding it up"
        );

        // Finish the part, and the parent follows without being touched.
        complete(&store, "auth-ux");
        let index = fresh_index(&store);
        let report = evaluate(&store, &index, "twin/app", "auth").unwrap();
        assert!(report.done);
        assert_eq!(report.blocked_by, None);
        assert_eq!(report.score(), (2, 2));
    }

    #[test]
    fn parts_nest_and_roll_up_through_every_level() {
        let (_dir, store) = world();
        for (slug, title) in [
            ("app", "The app"),
            ("app-ui", "Interface"),
            ("app-ui-forms", "Forms"),
        ] {
            add(&store, "twin/app", slug, title, "building").unwrap();
        }
        link(
            &store,
            "twin/app",
            "app-ui",
            PART_OF,
            &feature_sid("twin/app", "app"),
        )
        .unwrap();
        link(
            &store,
            "twin/app",
            "app-ui-forms",
            PART_OF,
            &feature_sid("twin/app", "app-ui"),
        )
        .unwrap();

        let index = fresh_index(&store);
        let report = evaluate(&store, &index, "twin/app", "app").unwrap();
        assert_eq!(report.parts.len(), 1);
        assert_eq!(report.parts[0].parts.len(), 1, "grandchildren survive");
        assert!(!report.done, "a leaf three levels down is not ready");

        complete(&store, "app-ui-forms");
        let index = fresh_index(&store);
        let report = evaluate(&store, &index, "twin/app", "app").unwrap();
        assert!(report.done, "readiness rolls all the way up");

        // Ancestry reads the chain back out, nearest first.
        let chain = ancestry(&store, &index, &feature_sid("twin/app", "app-ui-forms")).unwrap();
        assert_eq!(
            chain.iter().map(|(_, s)| s.as_str()).collect::<Vec<_>>(),
            vec!["app-ui", "app"]
        );
    }

    #[test]
    fn composition_refuses_the_shapes_that_are_not_trees() {
        let (_dir, store) = world();
        for slug in ["a", "b", "c"] {
            add(&store, "twin/app", slug, slug, "building").unwrap();
        }
        let sid = |s: &str| feature_sid("twin/app", s);

        // Itself.
        let err = link(&store, "twin/app", "a", PART_OF, &sid("a")).unwrap_err();
        assert!(err.to_string().contains("part of itself"), "{err}");

        // Something that is not a feature.
        let file = StableId::derive(&["file", "src/lib.rs"]);
        let err = link(&store, "twin/app", "a", PART_OF, &file).unwrap_err();
        assert!(err.to_string().contains("another feature"), "{err}");

        // A loop, at any distance.
        link(&store, "twin/app", "b", PART_OF, &sid("a")).unwrap();
        link(&store, "twin/app", "c", PART_OF, &sid("b")).unwrap();
        let err = link(&store, "twin/app", "a", PART_OF, &sid("c")).unwrap_err();
        assert!(err.to_string().contains("loop"), "{err}");

        // Two parents. The message says how to change your mind.
        add(&store, "twin/app", "d", "d", "building").unwrap();
        let err = link(&store, "twin/app", "c", PART_OF, &sid("d")).unwrap_err();
        assert!(err.to_string().contains("already part of 'b'"), "{err}");
        assert!(err.to_string().contains("relation retract"), "{err}");

        // Re-stating the same parent is idempotent, not an error.
        assert!(!link(&store, "twin/app", "c", PART_OF, &sid("b")).unwrap());
    }

    #[test]
    fn a_part_resolves_to_a_feature_even_when_a_decision_shares_its_slug() {
        let (_dir, store) = world();
        // adr-001-shape exists as a decision; register a feature with the
        // same slug and check the pinned resolver picks the right one.
        add(&store, "twin/app", "adr-001-shape", "Shape", "building").unwrap();
        let index = fresh_index(&store);

        let (_, kind) = resolve_target(&store, &index, "twin/app", "adr-001-shape")
            .unwrap()
            .unwrap();
        assert_eq!(kind, "decision", "unpinned resolution still prefers the ADR");

        let (sid, kind) =
            resolve_target_as(&store, &index, "twin/app", "adr-001-shape", Some("feature"))
                .unwrap()
                .unwrap();
        assert_eq!(kind, "feature");
        assert_eq!(sid, feature_sid("twin/app", "adr-001-shape"));
    }
}
