//! Reading the workspace into the graph: one pass per file, writing only what drifted.

use super::*;
use crate::agents::{self};
use crate::docs::{self};
use crate::symbols;
use crate::testing;
use brain_core::ids::{NodeId, StableId};
use brain_core::object::Object;
use brain_index::{replay, Index, MemIndex};
use brain_store::{now_ms, Store, StoreError};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

#[derive(Debug, Default, PartialEq, serde::Serialize)]
pub struct TwinReport {
    pub added: Vec<String>,
    pub changed: Vec<String>,
    pub deleted: Vec<String>,
    pub unchanged: usize,
    /// Symbol entities written this run (refresh only).
    pub symbols: usize,
    /// Relations written this run (refresh only).
    pub relations: usize,
    /// Edges retracted this run: structure the pass no longer observed.
    pub retracted: usize,
    /// Decision/plan documents captured this run (refresh only): rel paths.
    pub docs: Vec<String>,
}

/// Refresh the twin under `prefix` from the tree at `root`, writing only
/// what drifted. Idempotent: an immediately repeated refresh writes nothing.
pub fn refresh(store: &Store, root: &Path, prefix: &str) -> Result<TwinReport, StoreError> {
    run(store, root, prefix, true, false)
}

/// Like [`refresh`], but reprocesses every file as if it had changed —
/// the upgrade path after extractors improve (better import resolution,
/// new classifiers): existing files gain the new structure without
/// waiting to drift. Still guarded: unchanged facts write nothing.
pub fn refresh_full(store: &Store, root: &Path, prefix: &str) -> Result<TwinReport, StoreError> {
    run(store, root, prefix, true, true)
}

/// The same comparison as [`refresh`], read-only: what *would* be recorded?
pub fn status(store: &Store, root: &Path, prefix: &str) -> Result<TwinReport, StoreError> {
    run(store, root, prefix, false, false)
}

pub(crate) fn run(
    store: &Store,
    root: &Path,
    prefix: &str,
    write: bool,
    full: bool,
) -> Result<TwinReport, StoreError> {
    let mut index = MemIndex::new();
    let fed = replay(store, &mut index)?;
    let now = now_ms();
    let mut report = TwinReport::default();

    // The merged kind registry (compiled defaults ⊔ graph observations):
    // its capture rules route paths to kinds — the store teaching itself
    // new artifact types with no code change, and the shipped kinds
    // working before any seed.
    let registry = crate::kinds::registry(store, &index)?;
    let rules = crate::kinds::capture_rules(&registry);

    // Runtime-taught ingestion: repo-level extensions apply everywhere;
    // kind-level extensions only where that kind's globs reach.
    let mut extra = crate::ExtraIngest::default();
    let repo_sid_early = StableId::derive(&["repo", prefix]);
    if let Some(v) = latest(&index, store, &repo_sid_early, "ingest_extensions")? {
        extra.repo_exts.extend(
            v.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
        );
    }
    for def in registry.values() {
        if !def.extensions.is_empty() {
            let mut globs = def.capture.clone();
            globs.extend(def.home.iter().cloned());
            if !globs.is_empty() {
                extra
                    .kind_rules
                    .push((def.extensions.iter().cloned().collect(), globs));
            }
        }
    }

    let mut files = Vec::new();
    crate::collect_files_with(root, root, &extra, &mut files)?;
    files.sort();
    let file_set: BTreeSet<String> = files.iter().cloned().collect();

    let ns = store.namespace()?;
    let known: BTreeSet<String> = ns
        .keys()
        .filter_map(|n| n.strip_prefix(&format!("{prefix}/")))
        .map(str::to_string)
        .collect();

    let mut bindings: Vec<(String, NodeId)> = Vec::new();
    let mut written_relations: BTreeSet<(StableId, String, StableId)> = BTreeSet::new();
    // Content hashes of files first seen this run: the rename detector's
    // matching side (same-run delete + add of identical bytes).
    let mut added_by_hash: BTreeMap<String, Vec<String>> = BTreeMap::new();

    // Entity kinds governed by a seeded template: captured documents of
    // these kinds need a conformance pass even when otherwise unchanged.
    let tmpl_kinds: BTreeSet<String> = crate::templates::by_kind(store, &index)?
        .keys()
        .cloned()
        .collect();

    for rel in &files {
        let content = fs::read(root.join(rel))?;
        let hash = blake3::hash(&content).to_hex().to_string();
        let sid = StableId::derive(&["file", rel]);
        let prior = latest(&index, store, &sid, "content_b3")?;

        let changed = match &prior {
            None => {
                report.added.push(rel.clone());
                added_by_hash
                    .entry(hash.clone())
                    .or_default()
                    .push(rel.clone());
                true
            }
            Some(v) if *v != hash => {
                report.changed.push(rel.clone());
                true
            }
            _ => {
                report.unchanged += 1;
                false
            }
        };

        if !write {
            continue;
        }

        // A file that was marked deleted and is back gets its presence restored
        // even when its content is unchanged.
        if latest(&index, store, &sid, "present")?.as_deref() == Some("false") {
            observe(store, &sid, "present", "true", now)?;
        }

        let text = String::from_utf8_lossy(&content);
        let structure = symbols::analyze(rel, &text);
        // Backfill: an unchanged file twinned before structure extraction
        // existed (no `language` observation yet) still gets its structure.
        let structure_missing =
            !structure.language.is_empty() && latest(&index, store, &sid, "language")?.is_none();
        // A projection whose bytes match its render contract is a view of
        // an artifact that already owns the semantics — never re-capture
        // it as a second document.
        let is_projection =
            latest(&index, store, &sid, "expected_b3")?.as_deref() == Some(hash.as_str());
        // Same backfill rule for decision/plan documents twinned before doc
        // capture existed (no `content` observation on the doc entity yet).
        let doc_meta = if is_projection {
            None
        } else {
            docs::parse_doc(rel, &text)
        };
        let doc_missing = match &doc_meta {
            Some(m) => {
                let dsid = doc_sid(prefix, m);
                latest(&index, store, &dsid, "content")?.is_none()
                    || (tmpl_kinds.contains(m.kind.as_str())
                        && (latest(&index, store, &dsid, "conforms")?.is_none()
                            || latest(&index, store, &dsid, "template_b3")?.is_none()))
            }
            None => false,
        };
        // And for skills / agent configuration.
        let agent_meta = if is_projection {
            None
        } else {
            agents::parse_agent_doc(rel, &text)
        };
        let agent_missing = match &agent_meta {
            Some(a) => {
                let asid = agent_doc_sid(prefix, a);
                latest(&index, store, &asid, "content")?.is_none()
                    || (tmpl_kinds.contains(a.kind.as_str())
                        && (latest(&index, store, &asid, "conforms")?.is_none()
                            || latest(&index, store, &asid, "template_b3")?.is_none()))
            }
            None => false,
        };
        // And for test classification (files twinned before it existed).
        let test_info = testing::classify(rel, structure.language, &text);
        let test_missing =
            test_info.is_some() && latest(&index, store, &sid, "test_framework")?.is_none();
        // Registry rules capture paths the built-in detectors didn't
        // claim; built-ins keep precedence, most-specific pattern wins.
        let rule = if doc_meta.is_none() && agent_meta.is_none() && !is_projection {
            crate::kinds::match_rule(&rules, rel)
        } else {
            None
        };
        let rule_missing = match rule {
            Some(r) => {
                let rsid = StableId::derive(&[r.kind.as_str(), prefix, &docs::slug_of(rel)]);
                latest(&index, store, &rsid, "content")?.is_none()
                    || (tmpl_kinds.contains(r.kind.as_str())
                        && (latest(&index, store, &rsid, "conforms")?.is_none()
                            || latest(&index, store, &rsid, "template_b3")?.is_none()))
            }
            None => false,
        };
        if !full
            && !changed
            && !structure_missing
            && !doc_missing
            && !agent_missing
            && !test_missing
            && !rule_missing
        {
            continue;
        }

        if changed {
            let mut labels = BTreeMap::new();
            labels.insert("path".to_string(), rel.clone());
            let entity_node = store.put(&Object::Entity {
                id: sid.clone(),
                entity_kind: "source_file".to_string(),
                labels,
            })?;
            if prior.is_none() {
                bindings.push((format!("{prefix}/{rel}"), entity_node));
            }
            observe(store, &sid, "content_b3", &hash, now)?;
        }

        if !structure.language.is_empty()
            && latest(&index, store, &sid, "language")?.as_deref() != Some(structure.language)
        {
            observe(store, &sid, "language", structure.language, now)?;
        }

        for sym in &structure.symbols {
            let sym_sid = StableId::derive(&["symbol", rel, sym.kind, &sym.name]);
            let mut labels = BTreeMap::new();
            labels.insert("file".to_string(), rel.clone());
            labels.insert("kind".to_string(), sym.kind.to_string());
            labels.insert("name".to_string(), sym.name.clone());
            store.put(&Object::Entity {
                id: sym_sid.clone(),
                entity_kind: "symbol".to_string(),
                labels,
            })?;
            report.symbols += 1;
            if latest(&index, store, &sym_sid, "line")?.as_deref()
                != Some(sym.line.to_string().as_str())
            {
                observe(store, &sym_sid, "line", &sym.line.to_string(), now)?;
            }
            if relate(
                store,
                &index,
                &mut written_relations,
                &sid,
                "contains",
                &sym_sid,
                now,
            )? {
                report.relations += 1;
            }
        }

        for import in &structure.imports {
            let resolved = resolve_import(rel, import, &file_set);
            let target = match &resolved {
                Some(target_rel) => StableId::derive(&["file", target_rel]),
                None => {
                    let module_sid = StableId::derive(&["module", import]);
                    let mut labels = BTreeMap::new();
                    labels.insert("name".to_string(), import.clone());
                    store.put(&Object::Entity {
                        id: module_sid.clone(),
                        entity_kind: "module".to_string(),
                        labels,
                    })?;
                    module_sid
                }
            };
            if relate(
                store,
                &index,
                &mut written_relations,
                &sid,
                "imports",
                &target,
                now,
            )? {
                report.relations += 1;
            }
            // A test file covers the twinned files it imports.
            if resolved.is_some() && test_info.as_ref().is_some_and(|t| t.is_test_file) {
                if relate(
                    store,
                    &index,
                    &mut written_relations,
                    &sid,
                    "covers",
                    &target,
                    now,
                )? {
                    report.relations += 1;
                }
            }
        }

        // Test classification: framework, declared count, and role — so
        // "what is a test here" and "which tests cover this file" are
        // graph queries, not directory lore.
        if let Some(t) = &test_info {
            if latest(&index, store, &sid, "test_framework")?.as_deref() != Some(t.framework) {
                observe(store, &sid, "test_framework", t.framework, now)?;
            }
            let declared = t.declared.to_string();
            if latest(&index, store, &sid, "tests_declared")?.as_deref() != Some(declared.as_str())
            {
                observe(store, &sid, "tests_declared", &declared, now)?;
            }
            if t.is_test_file
                && latest(&index, store, &sid, "file_role")?.as_deref() != Some("test")
            {
                observe(store, &sid, "file_role", "test", now)?;
            }
        }

        // A file following an ADR/plan convention is also a *why* document:
        // capture it as a decision/plan entity beside its file entity.
        if let Some(meta) = &doc_meta {
            let out = record_doc(
                store,
                &index,
                prefix,
                meta,
                &text,
                "twin",
                Some(rel),
                &file_set,
                &mut written_relations,
                now,
            )?;
            report.relations += out.relations;
            report.retracted += out.retracted;
            if out.wrote {
                report.docs.push(rel.clone());
            }
        }

        // A skill or agent-configuration file is a *how it is built*
        // document: capture it the same way.
        if let Some(doc) = &agent_meta {
            let out = record_agent_doc(
                store,
                &index,
                prefix,
                doc,
                &text,
                "twin",
                Some(rel),
                &file_set,
                &mut written_relations,
                now,
            )?;
            report.relations += out.relations;
            report.retracted += out.retracted;
            if out.wrote {
                report.docs.push(rel.clone());
            }
        }

        // A file claimed by a graph-defined rule is an artifact of that
        // rule's kind: extracted fields become observations, and the
        // shared core supplies mentions/concerns/conformance unchanged.
        if let Some(r) = rule {
            let slug = docs::slug_of(rel);
            let extracted = r.extract(&text, &slug);
            let title = extracted
                .iter()
                .find(|(p, _)| p == "title")
                .map(|(_, v)| v.clone())
                .unwrap_or_else(|| slug.clone());
            let mut props: Vec<(&str, &str)> = vec![("content", &text)];
            for (p, v) in &extracted {
                if p != "content" {
                    props.push((p.as_str(), v.as_str()));
                }
            }
            let out = record_entity_doc(
                store,
                &index,
                prefix,
                &r.kind,
                &slug,
                &[("title", &title)],
                &props,
                &text,
                "twin",
                Some(rel),
                &file_set,
                &mut written_relations,
                now,
            )?;
            report.relations += out.relations;
            report.retracted += out.retracted;
            if out.wrote {
                report.docs.push(rel.clone());
            }
        }

        // Currency sweep: structure this pass did not re-observe has
        // vanished from the file — retract those edges so hubs, blast
        // radius, and totals track reality instead of history.
        report.retracted += sweep_edges(
            store,
            &index,
            &written_relations,
            &sid,
            &["contains", "imports", "covers"],
            now,
        )?;
    }

    // Files the twin still claims are present but which are gone from disk.
    let no_edges: BTreeSet<(StableId, String, StableId)> = BTreeSet::new();
    for rel in known.iter() {
        if file_set.contains(rel) {
            continue;
        }
        let sid = StableId::derive(&["file", rel]);
        let already = latest(&index, store, &sid, "present")?.as_deref() == Some("false");
        if !already {
            report.deleted.push(rel.clone());
        }
        if write {
            if !already {
                observe(store, &sid, "present", "false", now)?;
                // The vanished content reappeared verbatim at exactly one
                // new path this run: a move, not a death — leave the trail.
                if let Some(h) = latest(&index, store, &sid, "content_b3")? {
                    if let Some([new_rel]) = added_by_hash.get(&h).map(Vec::as_slice) {
                        let new_sid = StableId::derive(&["file", new_rel]);
                        if relate(
                            store,
                            &index,
                            &mut written_relations,
                            &sid,
                            "renamed_to",
                            &new_sid,
                            now,
                        )? {
                            report.relations += 1;
                        }
                    }
                }
            }
            // A deleted file has no structure: retract every outgoing edge.
            // Runs for already-deleted files too — the self-healing path for
            // stores whose deletions predate edge tombstones.
            report.retracted += sweep_edges(
                store,
                &index,
                &no_edges,
                &sid,
                &["contains", "imports", "covers"],
                now,
            )?;
        }
    }

    if write {
        let repo_sid = StableId::derive(&["repo", prefix]);
        let mut labels = BTreeMap::new();
        labels.insert("prefix".to_string(), prefix.to_string());
        let repo_node = store.put(&Object::Entity {
            id: repo_sid.clone(),
            entity_kind: "repo".to_string(),
            labels,
        })?;
        if !ns.contains_key(prefix) {
            bindings.push((prefix.to_string(), repo_node));
        }
        for (prop, value) in git_info(root) {
            if latest(&index, store, &repo_sid, &prop)?.as_deref() != Some(value.as_str()) {
                observe(store, &repo_sid, &prop, &value, now)?;
            }
        }
        // Where this twin was observed from, so read-side queries (wake)
        // can compare the working tree against the graph without guessing.
        // An observation, not a truth: the path is machine-local and
        // consumers must tolerate it no longer existing.
        if let Ok(abs) = fs::canonicalize(root) {
            let abs = abs.to_string_lossy().to_string();
            if latest(&index, store, &repo_sid, "root")?.as_deref() != Some(abs.as_str()) {
                observe(store, &repo_sid, "root", &abs, now)?;
            }
        }
        if !bindings.is_empty() {
            store.bind_many(bindings)?;
        }

        // Record the totals series on the repo entity: files/symbols/relations
        // over time, guarded so an unchanged codebase writes nothing. This is
        // what makes insights *continuous* — trends persist in the graph and
        // travel with replication.
        // Catch the index up with what this refresh just wrote, instead of
        // rebuilding it from nothing. `on_object` is idempotent and the log
        // only grows, so feeding the tail is the same answer for the price
        // of the delta — a refresh used to replay the whole store twice.
        let history = store.put_history_shared()?;
        for id in history[fed.min(history.len())..].iter() {
            let obj = store.get(id)?;
            index.on_object(id, &obj);
        }
        let ins = insights_with(store, &index, prefix)?;
        let totals = [
            ("files_present", ins.files),
            ("symbols_total", ins.symbols),
            ("relations_total", ins.relations),
        ];
        let mut any_changed = false;
        for (prop, value) in totals {
            if latest(&index, store, &repo_sid, prop)?.as_deref() != Some(value.to_string().as_str())
            {
                any_changed = true;
            }
        }
        // Write all three together so every series point is complete.
        if any_changed {
            for (prop, value) in totals {
                observe(store, &repo_sid, prop, &value.to_string(), now)?;
            }
        }
        record_quality(store, &index, prefix, &repo_sid, &ins, now, true)?;
        // The reflex arc: applied changes whose evidence has since
        // arrived settle themselves — nobody should have to type the
        // command for a verdict the graph can already derive.
        crate::govern::reconcile_applied(store, &index, prefix, now)?;
    }

    Ok(report)
}

/// Record one quality reading on the repo entity, guarded like the
/// growth totals: all six properties land together at the same moment,
/// and an unchanged picture writes nothing. `with_spine` decides whether
/// the claims count is measured fresh; without it the last measured
/// value is carried forward, keeping the point complete without
/// vouching for a number this pass did not take.
pub(crate) fn record_quality(
    store: &Store,
    index: &MemIndex,
    prefix: &str,
    repo_sid: &StableId,
    ins: &Insights,
    now: u64,
    with_spine: bool,
) -> Result<bool, StoreError> {
    let (tests_passed, tests_total) = match ins.last_run {
        Some((_, total, passed, _)) => (passed, total),
        None => (0, 0),
    };
    let features_done = ins.features.iter().filter(|f| f.done).count();
    let stale_warn = ins
        .stale_docs
        .iter()
        .filter(|d| d.severity == Severity::Warn)
        .count();
    let uncorroborated = if with_spine {
        crate::spine::build(store, index, prefix)?
            .uncorroborated()
            .len()
    } else {
        latest(index, store, repo_sid, "uncorroborated_total")?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    };
    let values = [
        ("tests_passed", tests_passed),
        ("tests_total", tests_total),
        ("features_done", features_done),
        ("features_total", ins.features.len()),
        ("stale_warn_total", stale_warn),
        ("uncorroborated_total", uncorroborated),
    ];
    let mut any_changed = false;
    for (prop, value) in values {
        if latest(index, store, repo_sid, prop)?.as_deref() != Some(value.to_string().as_str()) {
            any_changed = true;
        }
    }
    if any_changed {
        for (prop, value) in values {
            observe(store, repo_sid, prop, &value.to_string(), now)?;
        }
    }
    Ok(any_changed)
}

// ---------------------------------------------------------------------------
// Insights: a synthesized picture of the software under a twin prefix
// ---------------------------------------------------------------------------

/// Merge extensions into the repo's runtime ingestion allowlist
/// (`ingest_extensions` on the repo entity). Additive only — the compiled
/// list never shrinks. Returns the resulting csv when anything changed.
pub fn add_ingest_extensions(
    store: &Store,
    prefix: &str,
    exts: &[String],
) -> Result<Option<String>, StoreError> {
    let mut index = MemIndex::new();
    replay(store, &mut index)?;
    let repo_sid = StableId::derive(&["repo", prefix]);
    let current = latest(&index, store, &repo_sid, "ingest_extensions")?.unwrap_or_default();
    let mut set: BTreeSet<String> = current
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    let before = set.len();
    set.extend(
        exts.iter()
            .flat_map(|e| e.split(','))
            .map(|e| e.trim().trim_start_matches('.').to_lowercase())
            .filter(|e| !e.is_empty()),
    );
    if set.len() == before && !current.is_empty() {
        return Ok(None);
    }
    let csv = set.into_iter().collect::<Vec<_>>().join(",");
    observe_src(
        store,
        &repo_sid,
        "ingest_extensions",
        &csv,
        "agent",
        now_ms(),
    )?;
    Ok(Some(csv))
}
