//! The twin engine: continuous, drift-aware reflection of external software.
//!
//! `refresh` is the sense organ: it walks a source tree, compares reality
//! against the twin's latest claims, and records only what changed — new and
//! changed files get fresh observations, structure (symbols via
//! [`crate::symbols`]) and relations; vanished files get `present=false`;
//! unchanged files write nothing, so repeated refreshes do not grow the
//! graph. `status` runs the identical comparison read-only.
//!
//! Nothing is ever overwritten: the twin's history of a file is its
//! observation timeline, and "deleted" is itself just an observation.

use crate::agents::{self, AgentDoc};
use crate::docs::{self, DocMeta};
use crate::symbols;
use crate::testing;
use brain_core::ids::{NodeId, StableId};
use brain_core::object::Object;
use brain_index::{replay, Index, MemIndex};
use brain_store::{now_ms, Store, StoreError};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Default, PartialEq)]
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

fn run(
    store: &Store,
    root: &Path,
    prefix: &str,
    write: bool,
    full: bool,
) -> Result<TwinReport, StoreError> {
    let mut index = MemIndex::new();
    replay(store, &mut index)?;
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
        if !bindings.is_empty() {
            store.bind_many(bindings)?;
        }

        // Record the totals series on the repo entity: files/symbols/relations
        // over time, guarded so an unchanged codebase writes nothing. This is
        // what makes insights *continuous* — trends persist in the graph and
        // travel with replication.
        let mut post = MemIndex::new();
        replay(store, &mut post)?;
        let ins = insights_with(store, &post, prefix)?;
        let totals = [
            ("files_present", ins.files),
            ("symbols_total", ins.symbols),
            ("relations_total", ins.relations),
        ];
        let mut any_changed = false;
        for (prop, value) in totals {
            if latest(&post, store, &repo_sid, prop)?.as_deref() != Some(value.to_string().as_str())
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
    }

    Ok(report)
}

// ---------------------------------------------------------------------------
// Insights: a synthesized picture of the software under a twin prefix
// ---------------------------------------------------------------------------

/// How far along one feature is, counted in whichever terms it is judged.
///
/// A feature with parts is judged by its parts (ADR-028), so the fraction
/// has to come from `DoneReport::score` rather than from the feature's own
/// links. Reading the score off the link count made every parent report
/// what it happened to be linked to directly — the root of the spine said
/// `1/4` while its thirteen parts were all ready.
#[derive(Debug, Clone)]
pub struct FeatureProgress {
    pub slug: String,
    pub status: String,
    /// "3/4" of requirements, or "2/13" of parts.
    pub fraction: String,
    pub done: bool,
    /// Whether the fraction counts parts rather than requirements.
    pub by_parts: bool,
}

#[derive(Debug, Default)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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

#[derive(Debug, Clone, PartialEq)]
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

    // Growth series: pair up the repo entity's totals observations by time.
    let mut points: BTreeMap<u64, (usize, usize, usize)> = BTreeMap::new();
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
            let point = points.entry(observed_at_ms).or_insert((0, 0, 0));
            match property.as_str() {
                "files_present" => point.0 = n,
                "symbols_total" => point.1 = n,
                "relations_total" => point.2 = n,
                _ => {}
            }
        }
    }
    ins.series = points
        .into_iter()
        .filter(|(_, (f, s, r))| *f + *s + *r > 0)
        .map(|(at, (f, s, r))| (at, f, s, r))
        .collect();

    Ok(ins)
}

fn observe(
    store: &Store,
    subject: &StableId,
    property: &str,
    value: &str,
    at: u64,
) -> Result<NodeId, StoreError> {
    observe_src(store, subject, property, value, "twin", at)
}

pub(crate) fn observe_src(
    store: &Store,
    subject: &StableId,
    property: &str,
    value: &str,
    source: &str,
    at: u64,
) -> Result<NodeId, StoreError> {
    store.put(&Object::Observation {
        subject: subject.clone(),
        property: property.to_string(),
        value: value.to_string(),
        source: source.to_string(),
        observed_at_ms: at,
    })
}

/// Write a relation unless the graph (or this run) already has it live.
/// A relation whose edge was retracted is restored with an `active=true`
/// observation instead of a duplicate relation object.
pub(crate) fn relate(
    store: &Store,
    index: &MemIndex,
    written: &mut BTreeSet<(StableId, String, StableId)>,
    from: &StableId,
    kind: &str,
    to: &StableId,
    at: u64,
) -> Result<bool, StoreError> {
    let key = (from.clone(), kind.to_string(), to.clone());
    if written.contains(&key) {
        return Ok(false);
    }
    for id in index.relations_from(from, kind) {
        if let Object::Relation { to: t, .. } = store.get(&id)? {
            if &t == to {
                written.insert(key);
                if !brain_index::edge_active(index, store, from, kind, to)? {
                    observe(
                        store,
                        &brain_index::edge_sid(from, kind, to),
                        "active",
                        "true",
                        at,
                    )?;
                    return Ok(true);
                }
                return Ok(false);
            }
        }
    }
    store.put(&Object::Relation {
        from: from.clone(),
        predicate: kind.to_string(),
        to: to.clone(),
        source: "twin".to_string(),
        observed_at_ms: at,
    })?;
    written.insert(key);
    Ok(true)
}

/// Retract an edge: write `active=false` on its edge sid, guarded — a
/// no-op when the edge is already retracted or the relation never existed.
pub(crate) fn retract_edge(
    store: &Store,
    index: &MemIndex,
    from: &StableId,
    kind: &str,
    to: &StableId,
    source: &str,
    at: u64,
) -> Result<bool, StoreError> {
    let mut exists = false;
    for id in index.relations_from(from, kind) {
        if let Object::Relation { to: t, .. } = store.get(&id)? {
            if &t == to {
                exists = true;
                break;
            }
        }
    }
    if !exists || !brain_index::edge_active(index, store, from, kind, to)? {
        return Ok(false);
    }
    observe_src(
        store,
        &brain_index::edge_sid(from, kind, to),
        "active",
        "false",
        source,
        at,
    )?;
    Ok(true)
}

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

/// Agent-facing retraction: mark an edge as no longer holding (a wrong
/// `feature link`, a superseded association). The relation object stays —
/// history is never destroyed — but every reader stops seeing the edge.
pub fn retract(
    store: &Store,
    from: &StableId,
    kind: &str,
    to: &StableId,
) -> Result<bool, StoreError> {
    let mut index = MemIndex::new();
    replay(store, &mut index)?;
    retract_edge(store, &index, from, kind, to, "agent", now_ms())
}

/// Live outgoing edges of one predicate: (relation node, target sid).
/// Retracted edges are skipped — this is how every reader sees only the
/// relations that still hold.
pub fn live_from(
    index: &MemIndex,
    store: &Store,
    from: &StableId,
    kind: &str,
) -> Result<Vec<(NodeId, StableId)>, StoreError> {
    let mut out = Vec::new();
    for id in index.relations_from(from, kind) {
        if let Object::Relation { to, .. } = store.get(&id)? {
            if brain_index::edge_active(index, store, from, kind, &to)? {
                out.push((id, to));
            }
        }
    }
    Ok(out)
}

/// Live incoming edges of one predicate: (relation node, source sid).
pub fn live_to(
    index: &MemIndex,
    store: &Store,
    to: &StableId,
    kind: &str,
) -> Result<Vec<(NodeId, StableId)>, StoreError> {
    let mut out = Vec::new();
    for id in index.relations_to(to, kind) {
        if let Object::Relation { from, .. } = store.get(&id)? {
            if brain_index::edge_active(index, store, &from, kind, to)? {
                out.push((id, from));
            }
        }
    }
    Ok(out)
}

/// Retract live edges of `kinds` from `from` that this pass did not
/// re-observe (their key is absent from `written`). Guarded: an edge
/// already retracted writes nothing.
fn sweep_edges(
    store: &Store,
    index: &MemIndex,
    written: &BTreeSet<(StableId, String, StableId)>,
    from: &StableId,
    kinds: &[&str],
    at: u64,
) -> Result<usize, StoreError> {
    let mut retracted = 0;
    for kind in kinds {
        for (_, to) in live_from(index, store, from, kind)? {
            if !written.contains(&(from.clone(), kind.to_string(), to.clone()))
                && retract_edge(store, index, from, kind, &to, "twin", at)?
            {
                retracted += 1;
            }
        }
    }
    Ok(retracted)
}

/// A human-readable label for an entity: path, name, or slug — else the id.
pub fn sid_label(index: &MemIndex, store: &Store, sid: &StableId) -> String {
    for node in index.entity_nodes(sid) {
        if let Ok(Object::Entity { labels, .. }) = store.get(&node) {
            for key in ["path", "name", "slug"] {
                if let Some(v) = labels.get(key) {
                    return v.clone();
                }
            }
        }
    }
    sid.to_string()
}

/// Latest observation value for (subject, property), by observation time.
pub fn latest(
    index: &MemIndex,
    store: &Store,
    subject: &StableId,
    property: &str,
) -> Result<Option<String>, StoreError> {
    Ok(latest_at(index, store, subject, property)?.map(|(_, v)| v))
}

/// Like [`latest`], but also returns when the value was observed.
pub fn latest_at(
    index: &MemIndex,
    store: &Store,
    subject: &StableId,
    property: &str,
) -> Result<Option<(u64, String)>, StoreError> {
    latest_at_before(index, store, subject, property, u64::MAX)
}

/// Like [`latest_at`], but also returns who wrote the value — the seed
/// upgrade rule needs to distinguish shipped defaults from local edits.
pub fn latest_with_source(
    index: &MemIndex,
    store: &Store,
    subject: &StableId,
    property: &str,
) -> Result<Option<(u64, String, String)>, StoreError> {
    let mut best: Option<(u64, String, String)> = None;
    for id in index.observations_of(subject) {
        if let Object::Observation {
            property: p,
            value,
            source,
            observed_at_ms,
            ..
        } = store.get(&id)?
        {
            if p == property && best.as_ref().is_none_or(|(b, _, _)| observed_at_ms >= *b) {
                best = Some((observed_at_ms, value, source));
            }
        }
    }
    Ok(best)
}

/// Bi-temporal read: the newest observation at or before `t`. Every
/// observation carries its time, so "as of" is a filter, not a replay.
pub fn latest_at_before(
    index: &MemIndex,
    store: &Store,
    subject: &StableId,
    property: &str,
    t: u64,
) -> Result<Option<(u64, String)>, StoreError> {
    let mut best: Option<(u64, String)> = None;
    for id in index.observations_of(subject) {
        if let Object::Observation {
            property: p,
            value,
            observed_at_ms,
            ..
        } = store.get(&id)?
        {
            if p == property
                && observed_at_ms <= t
                && best.as_ref().is_none_or(|(b, _)| observed_at_ms >= *b)
            {
                best = Some((observed_at_ms, value));
            }
        }
    }
    Ok(best)
}

/// The twin as it was: files present under `prefix` as of `t`, with the
/// content hash each had at that moment.
pub fn files_at(
    store: &Store,
    index: &MemIndex,
    prefix: &str,
    t: u64,
) -> Result<Vec<(String, String)>, StoreError> {
    let mut out = Vec::new();
    for (name, node) in store.namespace()? {
        let Some(rel) = name.strip_prefix(&format!("{prefix}/")) else {
            continue;
        };
        let Ok(Object::Entity {
            id: sid,
            entity_kind,
            ..
        }) = store.get(&node)
        else {
            continue;
        };
        if entity_kind != "source_file" {
            continue;
        }
        let Some((_, hash)) = latest_at_before(index, store, &sid, "content_b3", t)? else {
            continue; // did not exist yet at t
        };
        if latest_at_before(index, store, &sid, "present", t)?
            .map(|(_, v)| v)
            .as_deref()
            == Some("false")
        {
            continue; // was deleted at t
        }
        out.push((rel.to_string(), hash));
    }
    Ok(out)
}

/// Best-effort resolution of an import string to a twinned file path.
fn resolve_import(from_rel: &str, import: &str, files: &BTreeSet<String>) -> Option<String> {
    if files.contains(import) {
        return Some(import.to_string());
    }
    // Rust intra-crate: `crate::foo::Bar` -> <src-root>/foo.rs or foo/mod.rs,
    // where the src root is the importing file's path up through "src/".
    // Item imports (`crate::helper`) fall back to the crate root (lib.rs).
    if let Some(rest) = import.strip_prefix("crate::") {
        let src_root = if let Some(p) = from_rel.rfind("/src/") {
            &from_rel[..p + 5]
        } else if from_rel.starts_with("src/") {
            "src/"
        } else {
            ""
        };
        if let Some(first) = rest.split("::").next() {
            for cand in [
                format!("{src_root}{first}.rs"),
                format!("{src_root}{first}/mod.rs"),
                format!("{src_root}lib.rs"),
            ] {
                if files.contains(&cand) {
                    return Some(cand);
                }
            }
        }
    }
    // Rust cross-crate: `foo_bar::mod::Item` resolves into a sibling
    // crate's src tree when one exists among the walked files (crate dirs
    // may use hyphens where imports use underscores). Item and bare-crate
    // imports fall back to the crate root — the honest answer for
    // `use foo_bar::Thing` and `use foo_bar::{a, b}`.
    if !import.contains('/') {
        let mut segs = import.split("::");
        let first = segs.next().unwrap_or("");
        let second = segs.next();
        if !matches!(first, "crate" | "super" | "self" | "std" | "core" | "alloc")
            && first.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !first.is_empty()
        {
            let hyphen = first.replace('_', "-");
            for f in files {
                let Some(src_root) = f.strip_suffix("lib.rs") else {
                    continue;
                };
                let dir = src_root
                    .strip_suffix("/src/")
                    .map(|d| d.rsplit('/').next().unwrap_or(d))
                    .unwrap_or("");
                if dir.is_empty() || (dir != first && dir != hyphen) {
                    continue;
                }
                if let Some(second) = second {
                    for cand in [
                        format!("{src_root}{second}.rs"),
                        format!("{src_root}{second}/mod.rs"),
                    ] {
                        if files.contains(&cand) {
                            return Some(cand);
                        }
                    }
                }
                return Some(f.clone());
            }
        }
    }
    if import.starts_with("./") || import.starts_with("../") {
        let dir = match from_rel.rsplit_once('/') {
            Some((d, _)) => d,
            None => "",
        };
        let joined = normalize(&if dir.is_empty() {
            import.to_string()
        } else {
            format!("{dir}/{import}")
        });
        for suffix in ["", ".js", ".ts", ".jsx", ".tsx", ".py", ".php", ".rs"] {
            let cand = format!("{joined}{suffix}");
            if files.contains(&cand) {
                return Some(cand);
            }
        }
        for idx in ["/index.js", "/index.ts"] {
            let cand = format!("{joined}{idx}");
            if files.contains(&cand) {
                return Some(cand);
            }
        }
    }
    None
}

/// Collapse `.` and `..` components in a relative path.
fn normalize(p: &str) -> String {
    let mut stack: Vec<&str> = Vec::new();
    for part in p.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                stack.pop();
            }
            other => stack.push(other),
        }
    }
    stack.join("/")
}

fn git_info(root: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (prop, args) in [
        ("git_commit", ["rev-parse", "HEAD"]),
        ("git_branch", ["rev-parse", "--abbrev-ref"]),
    ] {
        let mut cmd = Command::new("git");
        cmd.arg("-C").arg(root).args(args);
        if prop == "git_branch" {
            cmd.arg("HEAD");
        }
        if let Ok(o) = cmd.output() {
            if o.status.success() {
                let v = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if !v.is_empty() {
                    out.push((prop.to_string(), v));
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Notes: durable agent memory attached to any entity
// ---------------------------------------------------------------------------

pub fn add_note(store: &Store, subject: &StableId, text: &str) -> Result<NodeId, StoreError> {
    store.put(&Object::Observation {
        subject: subject.clone(),
        property: "note".to_string(),
        value: text.to_string(),
        source: "agent".to_string(),
        observed_at_ms: now_ms(),
    })
}

/// All notes on an entity, oldest first. Ordered by the event log rather
/// than by timestamp sorting: the log is chronological by construction, so
/// two notes written in the same millisecond keep their true order.
pub fn notes(
    index: &MemIndex,
    store: &Store,
    subject: &StableId,
) -> Result<Vec<(u64, String)>, StoreError> {
    // Sort the subject's own observations by their position in the feed,
    // rather than walking the feed looking for them. Same order, and it
    // costs the subject's handful of observations instead of the whole
    // log — which was being re-read and re-parsed once per subject.
    let order = store.put_position()?;
    let mut candidates: Vec<NodeId> = index.observations_of(subject);
    candidates.sort_by_key(|id| order.get(id).copied().unwrap_or(usize::MAX));

    let mut out = Vec::new();
    for id in candidates {
        if let Object::Observation {
            property,
            value,
            observed_at_ms,
            ..
        } = store.get(&id)?
        {
            if property == "note" {
                out.push((observed_at_ms, value));
            }
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Decisions and plans: the *why* documents, as first-class twin entities
// ---------------------------------------------------------------------------

/// Stable identity of a decision/plan under a prefix. Derived from kind +
/// prefix + slug, not from the file path: the document's identity survives
/// file moves, and out-of-repo documents get the same scheme.
pub fn doc_sid(prefix: &str, meta: &DocMeta) -> StableId {
    StableId::derive(&[meta.kind.as_str(), prefix, &meta.slug])
}

#[derive(Debug)]
pub struct DocOutcome {
    pub sid: StableId,
    /// Did this call write anything (observations or relations)?
    pub wrote: bool,
    /// Relations written by this call.
    pub relations: usize,
    /// Edges retracted by this call (mentions dropped from the text,
    /// recorded_in of a former location).
    pub retracted: usize,
    /// Twinned file paths this document mentions.
    pub mentions: Vec<String>,
}

/// Stable identity of a skill/agent-config document under a prefix.
pub fn agent_doc_sid(prefix: &str, doc: &AgentDoc) -> StableId {
    StableId::derive(&[doc.kind.as_str(), prefix, &doc.slug])
}

/// The shared recording core for semantic documents (decisions, plans,
/// skills, agent configuration). Every observation is guarded by [`latest`],
/// so re-recording an unchanged document writes nothing — and a changed
/// property value becomes a new observation: a timeline, never an overwrite.
#[allow(clippy::too_many_arguments)]
fn record_entity_doc(
    store: &Store,
    index: &MemIndex,
    prefix: &str,
    entity_kind: &str,
    slug: &str,
    extra_labels: &[(&str, &str)],
    props: &[(&str, &str)],
    content: &str,
    source: &str,
    rel_path: Option<&str>,
    candidates: &BTreeSet<String>,
    written_relations: &mut BTreeSet<(StableId, String, StableId)>,
    now: u64,
) -> Result<DocOutcome, StoreError> {
    let sid = StableId::derive(&[entity_kind, prefix, slug]);
    let mut labels = BTreeMap::new();
    labels.insert("prefix".to_string(), prefix.to_string());
    labels.insert("slug".to_string(), slug.to_string());
    for (k, v) in extra_labels {
        labels.insert(k.to_string(), v.to_string());
    }
    if let Some(rel) = rel_path {
        labels.insert("path".to_string(), rel.to_string());
    }
    store.put(&Object::Entity {
        id: sid.clone(),
        entity_kind: entity_kind.to_string(),
        labels,
    })?;

    let mut wrote = false;
    for (prop, value) in props {
        if latest(index, store, &sid, prop)?.as_deref() != Some(*value) {
            observe_src(store, &sid, prop, value, source, now)?;
            wrote = true;
        }
    }

    let mut outcome = DocOutcome {
        sid: sid.clone(),
        wrote,
        relations: 0,
        retracted: 0,
        mentions: Vec::new(),
    };

    let repo_sid = StableId::derive(&["repo", prefix]);
    if relate(
        store,
        index,
        written_relations,
        &sid,
        "concerns",
        &repo_sid,
        now,
    )? {
        outcome.relations += 1;
    }
    // Auto-detected documents keep their file entity too: the document is
    // the semantic thing, the file is where it happens to be recorded.
    if let Some(rel) = rel_path {
        let file_sid = StableId::derive(&["file", rel]);
        if relate(
            store,
            index,
            written_relations,
            &sid,
            "recorded_in",
            &file_sid,
            now,
        )? {
            outcome.relations += 1;
        }
    }
    // Mentions-scan: link the document to every twinned file its text names.
    for cand in candidates {
        if Some(cand.as_str()) == rel_path || !content.contains(cand.as_str()) {
            continue;
        }
        let file_sid = StableId::derive(&["file", cand]);
        if relate(
            store,
            index,
            written_relations,
            &sid,
            "mentions",
            &file_sid,
            now,
        )? {
            outcome.relations += 1;
        }
        outcome.mentions.push(cand.clone());
    }

    // Conformance against the graph-defined template for this kind, when
    // one is seeded: recorded as observations, never enforcement.
    if let Some((tmpl_sid, requires)) = crate::templates::by_kind(store, index)?.get(entity_kind) {
        let missing = crate::templates::check(content, requires);
        let conforms = if missing.is_empty() { "true" } else { "false" };
        if latest(index, store, &sid, "conforms")?.as_deref() != Some(conforms) {
            observe_src(store, &sid, "conforms", conforms, source, now)?;
            outcome.wrote = true;
        }
        let missing_val = missing.join(",");
        let prior = latest(index, store, &sid, "missing")?;
        if prior.as_deref() != Some(missing_val.as_str())
            && (!missing_val.is_empty() || prior.is_some())
        {
            observe_src(store, &sid, "missing", &missing_val, source, now)?;
            outcome.wrote = true;
        }
        if relate(
            store,
            index,
            written_relations,
            &sid,
            "conforms_to",
            tmpl_sid,
            now,
        )? {
            outcome.relations += 1;
        }
        // Version-precise conformance: record WHICH contract judged this
        // artifact, so template fitness can compare versions later.
        if let Some(contract) = latest(index, store, tmpl_sid, "contract_b3")? {
            if latest(index, store, &sid, "template_b3")?.as_deref() != Some(contract.as_str()) {
                observe_src(store, &sid, "template_b3", &contract, source, now)?;
                outcome.wrote = true;
            }
        }
    }

    // Currency sweep: a mention whose path the text no longer names is
    // retracted; a mention of a deleted file the text still names stays
    // live (that mismatch is a coherence finding, not stale structure).
    for (_, to) in live_from(index, store, &sid, "mentions")? {
        let path = sid_label(index, store, &to);
        if !content.contains(&path)
            && retract_edge(store, index, &sid, "mentions", &to, source, now)?
        {
            outcome.retracted += 1;
        }
    }
    // A moved document re-attaches to its new location; the old one is
    // retracted (doc identity is kind+prefix+slug, not the path).
    if let Some(rel) = rel_path {
        let here = StableId::derive(&["file", rel]);
        for (_, to) in live_from(index, store, &sid, "recorded_in")? {
            if to != here && retract_edge(store, index, &sid, "recorded_in", &to, source, now)? {
                outcome.retracted += 1;
            }
        }
    }

    if outcome.relations > 0 || outcome.retracted > 0 {
        outcome.wrote = true;
    }
    Ok(outcome)
}

/// Record a decision/plan document into the twin. A changed `Status:` line
/// becomes a new `status` observation: decisions get a timeline for free.
#[allow(clippy::too_many_arguments)]
pub fn record_doc(
    store: &Store,
    index: &MemIndex,
    prefix: &str,
    meta: &DocMeta,
    content: &str,
    source: &str,
    rel_path: Option<&str>,
    candidates: &BTreeSet<String>,
    written_relations: &mut BTreeSet<(StableId, String, StableId)>,
    now: u64,
) -> Result<DocOutcome, StoreError> {
    let mut props: Vec<(&str, &str)> = vec![("content", content), ("title", &meta.title)];
    if let Some(status) = &meta.status {
        props.push(("status", status));
    }
    let mut outcome = record_entity_doc(
        store,
        index,
        prefix,
        meta.kind.as_str(),
        &meta.slug,
        &[("title", &meta.title)],
        &props,
        content,
        source,
        rel_path,
        candidates,
        written_relations,
        now,
    )?;
    if let Some(other) = &meta.supersedes {
        let other_sid = StableId::derive(&["decision", prefix, other]);
        if relate(
            store,
            index,
            written_relations,
            &outcome.sid,
            "supersedes",
            &other_sid,
            now,
        )? {
            outcome.relations += 1;
            outcome.wrote = true;
        }
    }
    Ok(outcome)
}

/// Record a skill or agent-configuration document into the twin.
#[allow(clippy::too_many_arguments)]
pub fn record_agent_doc(
    store: &Store,
    index: &MemIndex,
    prefix: &str,
    doc: &AgentDoc,
    content: &str,
    source: &str,
    rel_path: Option<&str>,
    candidates: &BTreeSet<String>,
    written_relations: &mut BTreeSet<(StableId, String, StableId)>,
    now: u64,
) -> Result<DocOutcome, StoreError> {
    let mut props: Vec<(&str, &str)> = vec![
        ("content", content),
        ("name", &doc.name),
        ("agent", &doc.agent),
        ("role", &doc.role),
    ];
    if let Some(d) = &doc.description {
        props.push(("description", d));
    }
    record_entity_doc(
        store,
        index,
        prefix,
        doc.kind.as_str(),
        &doc.slug,
        &[
            ("name", &doc.name),
            ("agent", &doc.agent),
            ("role", &doc.role),
        ],
        &props,
        content,
        source,
        rel_path,
        candidates,
        written_relations,
        now,
    )
}

/// Graph-first authoring: record an artifact of any kind directly into
/// the graph, no source file. The kind's `fields` DSL extracts extra
/// properties; conformance is judged exactly as for captured files.
/// Rendering a projection from the recorded content is the caller's next
/// step for graph-first kinds.
pub fn author_artifact(
    store: &Store,
    prefix: &str,
    kind: &str,
    slug: &str,
    title: &str,
    content: &str,
    source: &str,
) -> Result<DocOutcome, StoreError> {
    let mut index = MemIndex::new();
    replay(store, &mut index)?;
    let candidates = twinned_paths(store, prefix)?;
    let mut written = BTreeSet::new();
    let registry = crate::kinds::registry(store, &index)?;
    let mut props: Vec<(String, String)> = vec![
        ("content".to_string(), content.to_string()),
        ("title".to_string(), title.to_string()),
    ];
    if let Some(def) = registry.get(kind) {
        if let Some(rule) = def.rule() {
            for (p, v) in rule.extract(content, slug) {
                if p != "content" && p != "title" {
                    props.push((p, v));
                }
            }
        }
    }
    let prop_refs: Vec<(&str, &str)> = props
        .iter()
        .map(|(p, v)| (p.as_str(), v.as_str()))
        .collect();
    record_entity_doc(
        store,
        &index,
        prefix,
        kind,
        slug,
        &[("title", title)],
        &prop_refs,
        content,
        source,
        None,
        &candidates,
        &mut written,
        now_ms(),
    )
}

/// Explicit ingestion for documents outside the observed tree — the path
/// for Claude Code plan files (`~/.claude/plans/*.md`) or pasted decisions.
pub fn add_doc(
    store: &Store,
    prefix: &str,
    meta: &DocMeta,
    content: &str,
    source: &str,
) -> Result<DocOutcome, StoreError> {
    let mut index = MemIndex::new();
    replay(store, &mut index)?;
    let candidates = twinned_paths(store, prefix)?;
    let mut written = BTreeSet::new();
    record_doc(
        store,
        &index,
        prefix,
        meta,
        content,
        source,
        None,
        &candidates,
        &mut written,
        now_ms(),
    )
}

/// Explicit ingestion for agent skills/configuration outside the observed
/// tree — user-level `~/.claude/skills`, `~/.claude/CLAUDE.md`, and the like.
pub fn add_agent_doc(
    store: &Store,
    prefix: &str,
    doc: &AgentDoc,
    content: &str,
    source: &str,
) -> Result<DocOutcome, StoreError> {
    let mut index = MemIndex::new();
    replay(store, &mut index)?;
    let candidates = twinned_paths(store, prefix)?;
    let mut written = BTreeSet::new();
    record_agent_doc(
        store,
        &index,
        prefix,
        doc,
        content,
        source,
        None,
        &candidates,
        &mut written,
        now_ms(),
    )
}

/// Rel paths of files twinned under a prefix, from the namespace.
pub fn twinned_paths(store: &Store, prefix: &str) -> Result<BTreeSet<String>, StoreError> {
    Ok(store
        .namespace()?
        .keys()
        .filter_map(|n| n.strip_prefix(&format!("{prefix}/")))
        .map(str::to_string)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, tempfile::TempDir) {
        let src = tempfile::tempdir().unwrap();
        fs::create_dir_all(src.path().join("src")).unwrap();
        fs::create_dir_all(src.path().join("web")).unwrap();
        fs::write(
            src.path().join("src/main.rs"),
            "use crate::util;\npub fn main() {}\nstruct Config;\n",
        )
        .unwrap();
        fs::write(src.path().join("src/util.rs"), "pub fn helper() {}\n").unwrap();
        fs::write(
            src.path().join("web/app.js"),
            "import { h } from './util';\nexport function render() {}\n",
        )
        .unwrap();
        fs::write(src.path().join("web/util.js"), "export function h() {}\n").unwrap();
        fs::write(
            src.path().join("model.php"),
            "<?php\nnamespace App;\nuse App\\Db;\nclass Model {\npublic function load() {}\n}\n",
        )
        .unwrap();
        fs::write(
            src.path().join("run.py"),
            "import os\ndef main():\n    pass\n",
        )
        .unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        (src, store_dir)
    }

    fn fresh_index(store: &Store) -> MemIndex {
        let mut index = MemIndex::new();
        replay(store, &mut index).unwrap();
        index
    }

    #[test]
    fn refresh_builds_structure_and_is_idempotent() {
        let (src, store_dir) = fixture();
        let store = Store::open(store_dir.path()).unwrap();

        let r1 = refresh(&store, src.path(), "twin/app").unwrap();
        assert_eq!(r1.added.len(), 6);
        assert!(
            r1.symbols >= 8,
            "symbols across four languages: {}",
            r1.symbols
        );
        assert!(r1.relations >= 8, "contains + imports: {}", r1.relations);

        // Rust intra-crate import resolved to the file, not a module stub.
        {
            let index = fresh_index(&store);
            let main = StableId::derive(&["file", "src/main.rs"]);
            let util_rs = StableId::derive(&["file", "src/util.rs"]);
            let rels = index.relations_from(&main, "imports");
            assert_eq!(rels.len(), 1);
            match store.get(&rels[0]).unwrap() {
                Object::Relation { to, .. } => assert_eq!(to, util_rs),
                other => panic!("expected relation, got {other:?}"),
            }
        }

        let index = fresh_index(&store);
        // Structure queries: app.js contains render, imports resolved to util.js.
        let app = StableId::derive(&["file", "web/app.js"]);
        let util = StableId::derive(&["file", "web/util.js"]);
        assert_eq!(index.relations_from(&app, "contains").len(), 1);
        let imports = index.relations_from(&app, "imports");
        assert_eq!(imports.len(), 1);
        match store.get(&imports[0]).unwrap() {
            Object::Relation { to, .. } => assert_eq!(to, util, "relative import resolved to file"),
            other => panic!("expected relation, got {other:?}"),
        }
        // Unresolved imports become module entities.
        let py = StableId::derive(&["file", "run.py"]);
        let os_mod = StableId::derive(&["module", "os"]);
        let py_imports = index.relations_from(&py, "imports");
        assert_eq!(py_imports.len(), 1);
        match store.get(&py_imports[0]).unwrap() {
            Object::Relation { to, .. } => assert_eq!(to, os_mod),
            other => panic!("expected relation, got {other:?}"),
        }

        // Idempotence: an immediate second refresh writes nothing.
        let before = store.count_objects().unwrap();
        let r2 = refresh(&store, src.path(), "twin/app").unwrap();
        assert_eq!(r2.unchanged, 6);
        assert!(r2.added.is_empty() && r2.changed.is_empty() && r2.deleted.is_empty());
        assert_eq!(r2.symbols + r2.relations, 0);
        assert_eq!(store.count_objects().unwrap(), before, "no graph growth");
    }

    #[test]
    fn drift_is_reported_readonly_then_recorded() {
        let (src, store_dir) = fixture();
        let store = Store::open(store_dir.path()).unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();

        fs::write(
            src.path().join("run.py"),
            "import sys\ndef main():\n    pass\n",
        )
        .unwrap();
        fs::remove_file(src.path().join("web/util.js")).unwrap();
        fs::write(src.path().join("new.rs"), "pub fn fresh() {}\n").unwrap();

        // status: reports the drift, writes nothing.
        let before = store.count_objects().unwrap();
        let s = status(&store, src.path(), "twin/app").unwrap();
        assert_eq!(s.changed, vec!["run.py".to_string()]);
        assert_eq!(s.deleted, vec!["web/util.js".to_string()]);
        assert_eq!(s.added, vec!["new.rs".to_string()]);
        assert_eq!(
            store.count_objects().unwrap(),
            before,
            "status is read-only"
        );

        // refresh: records it.
        let r = refresh(&store, src.path(), "twin/app").unwrap();
        assert_eq!(r.changed, vec!["run.py".to_string()]);
        assert_eq!(r.deleted, vec!["web/util.js".to_string()]);
        let index = fresh_index(&store);
        let util = StableId::derive(&["file", "web/util.js"]);
        assert_eq!(
            latest(&index, &store, &util, "present").unwrap().as_deref(),
            Some("false"),
            "deletion is an observation"
        );

        // Once recorded, the drift is gone: nothing further to report.
        let r2 = refresh(&store, src.path(), "twin/app").unwrap();
        assert!(r2.changed.is_empty() && r2.deleted.is_empty() && r2.added.is_empty());

        // The file returns: presence is restored on the next refresh.
        fs::write(src.path().join("web/util.js"), "export function h() {}\n").unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        let index = fresh_index(&store);
        assert_eq!(
            latest(&index, &store, &util, "present").unwrap().as_deref(),
            Some("true")
        );
    }

    #[test]
    fn insights_synthesize_churn_hubs_and_growth_series() {
        let (src, store_dir) = fixture();
        let store = Store::open(store_dir.path()).unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();

        let ins = insights(&store, "twin/app").unwrap();
        assert_eq!(ins.files, 6);
        assert_eq!(ins.deleted_files, 0);
        assert!(ins.symbols >= 8);
        assert!(ins.external_modules.iter().any(|(m, _)| m == "os"));
        // app.js imports util.js; main.rs imports util.rs -> both are hubs.
        assert!(ins.hubs.iter().any(|(f, n)| f == "web/util.js" && *n == 1));
        assert!(ins.hubs.iter().any(|(f, n)| f == "src/util.rs" && *n == 1));
        assert!(ins.churn.is_empty(), "nothing edited yet");
        assert_eq!(ins.series.len(), 1, "first totals point recorded");

        // Edit a file twice across refreshes: churn appears, series grows.
        fs::write(
            src.path().join("run.py"),
            "import os\ndef main():\n    return 1\n",
        )
        .unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        fs::write(
            src.path().join("run.py"),
            "import os\ndef main():\n    return 2\ndef extra():\n    pass\n",
        )
        .unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();

        let ins = insights(&store, "twin/app").unwrap();
        assert!(
            ins.churn.iter().any(|(f, n)| f == "run.py" && *n == 3),
            "churn should count content versions: {:?}",
            ins.churn
        );
        assert!(ins.series.len() >= 2, "symbol growth adds a series point");

        // Idempotent refresh adds no series point.
        let points = ins.series.len();
        refresh(&store, src.path(), "twin/app").unwrap();
        let ins = insights(&store, "twin/app").unwrap();
        assert_eq!(ins.series.len(), points);

        // Notes surface in insights.
        let sid = StableId::derive(&["file", "run.py"]);
        add_note(&store, &sid, "agent: rewrote main twice while iterating").unwrap();
        let ins = insights(&store, "twin/app").unwrap();
        assert!(ins
            .notes
            .iter()
            .any(|(_, e, t)| e == "run.py" && t.contains("rewrote")));
    }

    #[test]
    fn decisions_and_plans_are_captured_and_linked() {
        let (src, store_dir) = fixture();
        let store = Store::open(store_dir.path()).unwrap();
        fs::create_dir_all(src.path().join("docs/adr")).unwrap();
        fs::create_dir_all(src.path().join("docs/plans")).unwrap();
        fs::write(
            src.path().join("docs/adr/adr-001-storage.md"),
            "# Use content addressing\n\nStatus: proposed\n\nAffects src/main.rs directly.\n",
        )
        .unwrap();
        fs::write(
            src.path().join("docs/plans/plan-v1.md"),
            "# Plan v1\n\nRefactor src/util.rs and web/app.js.\n",
        )
        .unwrap();

        let r = refresh(&store, src.path(), "twin/app").unwrap();
        assert_eq!(r.docs.len(), 2, "both documents captured: {:?}", r.docs);

        let index = fresh_index(&store);
        let adr = StableId::derive(&["decision", "twin/app", "adr-001-storage"]);
        let plan = StableId::derive(&["plan", "twin/app", "plan-v1"]);
        assert_eq!(
            latest(&index, &store, &adr, "status").unwrap().as_deref(),
            Some("proposed")
        );
        assert_eq!(
            latest(&index, &store, &adr, "title").unwrap().as_deref(),
            Some("Use content addressing")
        );
        assert!(latest(&index, &store, &plan, "content")
            .unwrap()
            .unwrap()
            .contains("Refactor"));

        // Linked: mentions -> the file it names, concerns -> repo,
        // recorded_in -> the markdown file entity.
        let main_sid = StableId::derive(&["file", "src/main.rs"]);
        let rels = index.relations_from(&adr, "mentions");
        assert_eq!(rels.len(), 1);
        match store.get(&rels[0]).unwrap() {
            Object::Relation { to, .. } => assert_eq!(to, main_sid),
            other => panic!("expected relation, got {other:?}"),
        }
        assert_eq!(index.relations_from(&adr, "concerns").len(), 1);
        assert_eq!(index.relations_from(&adr, "recorded_in").len(), 1);
        assert_eq!(index.relations_from(&plan, "mentions").len(), 2);

        // Idempotence: an immediate second refresh writes nothing.
        let before = store.count_objects().unwrap();
        let r2 = refresh(&store, src.path(), "twin/app").unwrap();
        assert!(r2.docs.is_empty());
        assert_eq!(store.count_objects().unwrap(), before, "no graph growth");

        // A status change is a new observation: decisions get a timeline.
        fs::write(
            src.path().join("docs/adr/adr-001-storage.md"),
            "# Use content addressing\n\nStatus: accepted\n\nAffects src/main.rs directly.\n",
        )
        .unwrap();
        // And a superseding decision links to what it replaces.
        fs::write(
            src.path().join("docs/adr/adr-002-sync.md"),
            "# Sync differently\n\nStatus: proposed\nSupersedes: adr-001-storage.md\n",
        )
        .unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        let index = fresh_index(&store);
        assert_eq!(
            latest(&index, &store, &adr, "status").unwrap().as_deref(),
            Some("accepted")
        );
        let statuses = index
            .observations_of(&adr)
            .iter()
            .filter_map(|id| store.get(id).ok())
            .filter(|o| matches!(o, Object::Observation { property, .. } if property == "status"))
            .count();
        assert_eq!(statuses, 2, "status history is a timeline");
        let adr2 = StableId::derive(&["decision", "twin/app", "adr-002-sync"]);
        let sup = index.relations_from(&adr2, "supersedes");
        assert_eq!(sup.len(), 1);
        match store.get(&sup[0]).unwrap() {
            Object::Relation { to, .. } => assert_eq!(to, adr),
            other => panic!("expected relation, got {other:?}"),
        }

        // Insights surface only the living decision set: the superseded ADR
        // is history, and files it alone mentioned lose their decided tag.
        let ins = insights(&store, "twin/app").unwrap();
        assert!(
            !ins.decisions.iter().any(|(s, _, _)| s == "adr-001-storage"),
            "superseded decisions leave the list: {:?}",
            ins.decisions
        );
        assert!(ins
            .decisions
            .iter()
            .any(|(s, _, st)| s == "adr-002-sync" && st == "proposed"));
        assert!(ins.plans.iter().any(|(s, _)| s == "plan-v1"));
        assert!(
            !ins.decided.contains("src/main.rs"),
            "its rationale was superseded"
        );
        assert!(!ins.decided.contains("run.py"));
        let (lc, why) = crate::lifecycle::of(&index, &store, &adr).unwrap();
        assert_eq!(lc, crate::lifecycle::Lifecycle::Superseded);
        assert!(why.contains("adr-002-sync"), "{why}");
    }

    #[test]
    fn skills_and_agent_config_are_captured() {
        let (src, store_dir) = fixture();
        let store = Store::open(store_dir.path()).unwrap();
        fs::create_dir_all(src.path().join(".claude/skills/deploy")).unwrap();
        fs::create_dir_all(src.path().join(".claude/agents")).unwrap();
        fs::write(
            src.path().join(".claude/skills/deploy/SKILL.md"),
            "---\nname: deploy\ndescription: Ship src/main.rs safely\n---\n\n# Deploy\n",
        )
        .unwrap();
        fs::write(
            src.path().join("CLAUDE.md"),
            "# Project rules\n\nStart at src/main.rs.\n",
        )
        .unwrap();
        fs::write(
            src.path().join(".claude/agents/reviewer.md"),
            "---\nname: reviewer\ndescription: Reviews diffs\n---\nReview carefully.\n",
        )
        .unwrap();
        fs::write(src.path().join(".cursorrules"), "Prefer small functions.\n").unwrap();

        let r = refresh(&store, src.path(), "twin/app").unwrap();
        assert_eq!(r.docs.len(), 4, "all agent docs captured: {:?}", r.docs);

        let index = fresh_index(&store);
        let skill = StableId::derive(&["skill", "twin/app", "deploy"]);
        assert_eq!(
            latest(&index, &store, &skill, "description")
                .unwrap()
                .as_deref(),
            Some("Ship src/main.rs safely")
        );
        assert_eq!(
            latest(&index, &store, &skill, "agent").unwrap().as_deref(),
            Some("claude")
        );
        // The skill mentions src/main.rs (from its description in content).
        let main_sid = StableId::derive(&["file", "src/main.rs"]);
        let mentioned: Vec<_> = index
            .relations_from(&skill, "mentions")
            .iter()
            .filter_map(|id| match store.get(id) {
                Ok(Object::Relation { to, .. }) => Some(to),
                _ => None,
            })
            .collect();
        assert_eq!(mentioned, vec![main_sid]);

        let claude_md = StableId::derive(&["agent_config", "twin/app", "claude.md"]);
        assert_eq!(
            latest(&index, &store, &claude_md, "role")
                .unwrap()
                .as_deref(),
            Some("instructions")
        );
        let reviewer = StableId::derive(&["agent_config", "twin/app", "reviewer"]);
        assert_eq!(
            latest(&index, &store, &reviewer, "role")
                .unwrap()
                .as_deref(),
            Some("subagent")
        );
        let cursor = StableId::derive(&["agent_config", "twin/app", ".cursorrules"]);
        assert_eq!(
            latest(&index, &store, &cursor, "agent").unwrap().as_deref(),
            Some("cursor")
        );

        // Idempotence still holds with agent docs present.
        let before = store.count_objects().unwrap();
        let r2 = refresh(&store, src.path(), "twin/app").unwrap();
        assert!(r2.docs.is_empty());
        assert_eq!(store.count_objects().unwrap(), before, "no graph growth");

        // Insights surface them.
        let ins = insights(&store, "twin/app").unwrap();
        assert!(ins
            .skills
            .iter()
            .any(|(s, a, d)| s == "deploy" && a == "claude" && d.contains("Ship")));
        assert!(ins
            .agent_configs
            .iter()
            .any(|(s, _, r)| s == "claude.md" && r == "instructions"));
        assert!(ins
            .agent_configs
            .iter()
            .any(|(s, _, r)| s == "reviewer" && r == "subagent"));

        // Explicit add for an out-of-repo skill (user-level ~/.claude).
        let content = "---\nname: triage\ndescription: Sort issues\n---\nSteps.\n";
        let doc = agents::parse_agent_doc("home/.claude/skills/triage/SKILL.md", content).unwrap();
        let out = add_agent_doc(&store, "twin/app", &doc, content, "claude-code").unwrap();
        assert!(out.wrote);
        let again = add_agent_doc(&store, "twin/app", &doc, content, "claude-code").unwrap();
        assert!(
            !again.wrote,
            "explicit re-add of unchanged skill writes nothing"
        );
        let ins = insights(&store, "twin/app").unwrap();
        assert!(ins.skills.iter().any(|(s, _, _)| s == "triage"));
    }

    #[test]
    fn explicit_add_doc_records_out_of_repo_plans() {
        let (src, store_dir) = fixture();
        let store = Store::open(store_dir.path()).unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();

        // A plan file living outside the observed tree (e.g. ~/.claude/plans).
        let content = "# The session plan\n\nTouch src/main.rs and run.py.\n";
        let meta = docs::parse_content(docs::DocKind::Plan, "session-plan", content, None, None);
        let out = add_doc(&store, "twin/app", &meta, content, "claude-code").unwrap();
        assert!(out.wrote);
        assert_eq!(
            out.mentions,
            vec!["run.py".to_string(), "src/main.rs".to_string()]
        );

        // Re-adding the identical document writes nothing.
        let before = store.count_objects().unwrap();
        let again = add_doc(&store, "twin/app", &meta, content, "claude-code").unwrap();
        assert!(!again.wrote);
        assert_eq!(store.count_objects().unwrap(), before);

        // The observations carry the explicit source, and insights list it.
        let index = fresh_index(&store);
        assert!(index.observations_of(&out.sid).iter().any(|id| matches!(
            store.get(id).unwrap(),
            Object::Observation { source, .. } if source == "claude-code"
        )));
        let ins = insights(&store, "twin/app").unwrap();
        assert!(ins
            .plans
            .iter()
            .any(|(s, t)| s == "session-plan" && t == "The session plan"));
    }

    #[test]
    fn templates_record_conformance_and_features_evaluate_done() {
        let (src, store_dir) = fixture();
        let store = Store::open(store_dir.path()).unwrap();
        crate::templates::seed(&store).unwrap();
        fs::create_dir_all(src.path().join("docs/adr")).unwrap();
        // One ADR honors the contract, one is missing its status.
        fs::write(
            src.path().join("docs/adr/adr-001-good.md"),
            "# Good decision\n\nStatus: accepted\n\nBecause src/main.rs needed it.\n",
        )
        .unwrap();
        fs::write(
            src.path().join("docs/adr/adr-002-bare.md"),
            "prose without contract\n",
        )
        .unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();

        let index = fresh_index(&store);
        let good = StableId::derive(&["decision", "twin/app", "adr-001-good"]);
        let bare = StableId::derive(&["decision", "twin/app", "adr-002-bare"]);
        assert_eq!(
            latest(&index, &store, &good, "conforms")
                .unwrap()
                .as_deref(),
            Some("true")
        );
        assert_eq!(
            latest(&index, &store, &bare, "conforms")
                .unwrap()
                .as_deref(),
            Some("false")
        );
        assert_eq!(
            latest(&index, &store, &bare, "missing").unwrap().as_deref(),
            Some("title,status")
        );
        assert_eq!(index.relations_from(&good, "conforms_to").len(), 1);

        // Insights surface the violation; fixing the file clears it.
        let ins = insights(&store, "twin/app").unwrap();
        assert!(ins
            .nonconforming
            .iter()
            .any(|(s, k, m)| { s == "adr-002-bare" && k == "decision" && m.contains("status") }));
        fs::write(
            src.path().join("docs/adr/adr-002-bare.md"),
            "# Now titled\n\nStatus: proposed\n\nprose with contract\n",
        )
        .unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        let index = fresh_index(&store);
        assert_eq!(
            latest(&index, &store, &bare, "conforms")
                .unwrap()
                .as_deref(),
            Some("true")
        );
        let ins = insights(&store, "twin/app").unwrap();
        assert!(ins.nonconforming.is_empty());

        // Refresh stays idempotent with templates seeded.
        let before = store.count_objects().unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        assert_eq!(store.count_objects().unwrap(), before, "no graph growth");

        // Feature registry: register, link, evaluate done against the DoD.
        let (fsid, wrote) =
            crate::features::add(&store, "twin/app", "render", "Rendering", "building").unwrap();
        assert!(wrote);
        let index = fresh_index(&store);
        let (main_sid, kind) =
            crate::features::resolve_target(&store, &index, "twin/app", "src/main.rs")
                .unwrap()
                .unwrap();
        assert_eq!(kind, "file");
        crate::features::link(&store, "twin/app", "render", "implemented_by", &main_sid).unwrap();
        let (adr_sid, kind) =
            crate::features::resolve_target(&store, &index, "twin/app", "adr-001-good")
                .unwrap()
                .unwrap();
        assert_eq!(kind, "decision");
        crate::features::link(&store, "twin/app", "render", "decided_by", &adr_sid).unwrap();

        let index = fresh_index(&store);
        let report = crate::features::evaluate(&store, &index, "twin/app", "render").unwrap();
        assert!(!report.done, "2 of 4 DoD predicates met");
        assert_eq!(
            report.checks.len(),
            4,
            "DoD comes from the seeded feature template"
        );
        assert_eq!(report.checks.iter().filter(|c| c.count > 0).count(), 2);
        assert!(
            crate::features::record_done(&store, &index, "twin/app", "render", &report).unwrap()
        );
        let index = fresh_index(&store);
        assert!(
            !crate::features::record_done(&store, &index, "twin/app", "render", &report).unwrap(),
            "unchanged done state writes nothing"
        );
        assert_eq!(
            latest(&index, &store, &fsid, "done").unwrap().as_deref(),
            Some("false")
        );

        // Complete the DoD: the feature flips to done.
        let test_sid = StableId::derive(&["file", "run.py"]);
        crate::features::link(&store, "twin/app", "render", "tested_by", &test_sid).unwrap();
        let readme = StableId::derive(&["file", "web/app.js"]);
        crate::features::link(&store, "twin/app", "render", "documented_in", &readme).unwrap();
        let index = fresh_index(&store);
        let report = crate::features::evaluate(&store, &index, "twin/app", "render").unwrap();
        assert!(report.done);

        // Insights render the matrix fraction.
        let ins = insights(&store, "twin/app").unwrap();
        assert!(ins
            .features
            .iter()
            .any(|f| f.slug == "render" && f.status == "building" && f.fraction == "4/4"));

        // A parent is judged by its parts (ADR-028), and insights must say
        // so too. Reading the fraction off the parent's own links made the
        // root of a spine report what it happened to be linked to directly
        // while every part under it was ready.
        crate::features::add(&store, "twin/app", "surface", "Surface", "building").unwrap();
        let parent = crate::features::feature_sid("twin/app", "surface");
        crate::features::link(&store, "twin/app", "render", "part_of", &parent).unwrap();
        let index = fresh_index(&store);
        let report = crate::features::evaluate(&store, &index, "twin/app", "surface").unwrap();
        assert!(report.by_parts() && report.done, "its one part is ready");
        assert_eq!(report.checks.iter().filter(|c| c.count > 0).count(), 0);

        let ins = insights(&store, "twin/app").unwrap();
        let surface = ins.features.iter().find(|f| f.slug == "surface").unwrap();
        assert!(surface.by_parts);
        assert!(surface.done, "the part is ready, so the parent is");
        assert_eq!(
            surface.fraction, "1/1",
            "the fraction counts parts, not the parent's own links"
        );
    }

    #[test]
    fn tests_classify_cover_and_protocols_form_timelines() {
        let (src, store_dir) = fixture();
        let store = Store::open(store_dir.path()).unwrap();
        fs::write(
            src.path().join("web/app.test.js"),
            "import { render } from './app';\ntest('renders', () => {});\nit('updates', () => {});\n",
        )
        .unwrap();
        fs::write(
            src.path().join("src/calc.rs"),
            "pub fn add(a: i64, b: i64) -> i64 { a + b }\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn t_add() {}\n}\n",
        )
        .unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();

        let index = fresh_index(&store);
        let spec = StableId::derive(&["file", "web/app.test.js"]);
        let app = StableId::derive(&["file", "web/app.js"]);
        let calc = StableId::derive(&["file", "src/calc.rs"]);
        assert_eq!(
            latest(&index, &store, &spec, "test_framework")
                .unwrap()
                .as_deref(),
            Some("jest")
        );
        assert_eq!(
            latest(&index, &store, &spec, "tests_declared")
                .unwrap()
                .as_deref(),
            Some("2")
        );
        assert_eq!(
            latest(&index, &store, &spec, "file_role")
                .unwrap()
                .as_deref(),
            Some("test")
        );
        // The spec covers the file it imports; inline Rust tests classify
        // the file without marking it role=test.
        assert_eq!(index.relations_to(&app, "covers").len(), 1);
        assert_eq!(
            latest(&index, &store, &calc, "test_framework")
                .unwrap()
                .as_deref(),
            Some("rust")
        );
        assert_eq!(latest(&index, &store, &calc, "file_role").unwrap(), None);

        // Refresh stays idempotent with test classification present.
        let before = store.count_objects().unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        assert_eq!(store.count_objects().unwrap(), before, "no graph growth");

        // Protocol 1: a cargo run with one failure.
        let run1 = "test calc::tests::t_add ... ok\ntest web::render ... FAILED\n";
        let report = testing::parse_report(run1);
        let out = testing::record_run(&store, "twin/app", &report, run1).unwrap();
        assert!(out.wrote);
        assert_eq!((out.total, out.passed, out.failed), (2, 1, 1));
        assert_eq!(out.failing, vec!["web::render".to_string()]);
        assert_eq!(out.transitions, 0, "first observations are not transitions");

        // Re-importing the identical report writes nothing.
        let before = store.count_objects().unwrap();
        let again = testing::record_run(&store, "twin/app", &report, run1).unwrap();
        assert!(!again.wrote);
        assert_eq!(store.count_objects().unwrap(), before);

        // The failing case is queryable, and the run left Behavioral
        // evidence on the repo entity.
        let index = fresh_index(&store);
        assert_eq!(
            testing::failing_cases(&store, &index, "twin/app").unwrap(),
            vec!["web::render".to_string()]
        );
        let repo_sid = StableId::derive(&["repo", "twin/app"]);
        let repo_node = index.entity_nodes(&repo_sid)[0];
        let evidence = index.evidence_for(&repo_node);
        assert_eq!(evidence.len(), 1);
        match store.get(&evidence[0]).unwrap() {
            Object::Evidence { passed, level, .. } => {
                assert!(!passed);
                assert_eq!(level, brain_core::object::VerificationLevel::Behavioral);
            }
            other => panic!("expected evidence, got {other:?}"),
        }

        // Protocol 2: the failure is fixed — a pass->fail->pass timeline.
        let run2 = "test calc::tests::t_add ... ok\ntest web::render ... ok\n";
        let out =
            testing::record_run(&store, "twin/app", &testing::parse_report(run2), run2).unwrap();
        assert!(out.wrote);
        assert_eq!(out.transitions, 1, "fail -> pass is a recorded transition");
        let index = fresh_index(&store);
        assert!(testing::failing_cases(&store, &index, "twin/app")
            .unwrap()
            .is_empty());
        assert_eq!(testing::runs(&store, &index, "twin/app").unwrap().len(), 2);

        // A JUnit (Playwright-style) run links cases to their spec file.
        let junit = "<testsuite>\n  <testcase classname=\"web/app.test.js\" name=\"renders\"/>\n</testsuite>\n";
        testing::record_run(&store, "twin/app", &testing::parse_report(junit), junit).unwrap();
        let index = fresh_index(&store);
        let case = StableId::derive(&["test", "twin/app", "web/app.test.js::renders"]);
        assert_eq!(index.relations_from(&case, "defined_in").len(), 1);

        // Insights: totals, last run, and the untested hub (src/util.rs is
        // imported but has no tests and no covering spec; web/app.js is
        // covered by the spec).
        let ins = insights(&store, "twin/app").unwrap();
        assert_eq!(ins.test_files, 1);
        assert!(ins.tests_declared >= 3);
        let (_, total, passed, failed) = ins.last_run.unwrap();
        assert_eq!((total, passed, failed), (1, 1, 0));
        assert!(ins.failing.is_empty());
        assert!(ins.untested_hubs.iter().any(|(f, _)| f == "src/util.rs"));
        assert!(!ins.untested_hubs.iter().any(|(f, _)| f == "web/app.js"));
    }

    #[test]
    fn docs_go_stale_when_mentioned_files_change_after_them() {
        let (src, store_dir) = fixture();
        let store = Store::open(store_dir.path()).unwrap();
        fs::create_dir_all(src.path().join("docs/adr")).unwrap();
        fs::write(
            src.path().join("docs/adr/adr-001-main.md"),
            "# Main design\n\nStatus: accepted\n\nAll logic lives in src/main.rs.\n",
        )
        .unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        let ins = insights(&store, "twin/app").unwrap();
        assert!(
            ins.stale_docs.is_empty(),
            "freshly captured doc is not stale"
        );

        // The mentioned file changes after the doc was recorded.
        std::thread::sleep(std::time::Duration::from_millis(5));
        fs::write(
            src.path().join("src/main.rs"),
            "use crate::util;\npub fn main() { util::helper() }\nstruct Config;\n",
        )
        .unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        let ins = insights(&store, "twin/app").unwrap();
        assert_eq!(ins.stale_docs.len(), 1);
        let d = &ins.stale_docs[0];
        assert_eq!(d.slug, "adr-001-main");
        assert_eq!(d.kind, "decision");
        assert_eq!(
            d.severity,
            Severity::Info,
            "decisions are records: info by default"
        );
        assert_eq!(d.changed, vec!["src/main.rs".to_string()]);

        // Acknowledging resets the clock without touching the file.
        let adr = StableId::derive(&["decision", "twin/app", "adr-001-main"]);
        std::thread::sleep(std::time::Duration::from_millis(5));
        ack(&store, &adr, "checked against current code").unwrap();
        let ins = insights(&store, "twin/app").unwrap();
        assert!(
            ins.stale_docs.is_empty(),
            "acknowledged doc is fresh, file untouched"
        );

        // A later change makes it stale again; updating the doc clears it.
        std::thread::sleep(std::time::Duration::from_millis(5));
        fs::write(
            src.path().join("src/main.rs"),
            "use crate::util;\npub fn main() { util::helper() }\n",
        )
        .unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        assert_eq!(insights(&store, "twin/app").unwrap().stale_docs.len(), 1);
        std::thread::sleep(std::time::Duration::from_millis(5));
        fs::write(
            src.path().join("docs/adr/adr-001-main.md"),
            "# Main design\n\nStatus: accepted\n\nAll logic lives in src/main.rs; helper moved in.\n",
        )
        .unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        let ins = insights(&store, "twin/app").unwrap();
        assert!(ins.stale_docs.is_empty(), "re-touched doc is fresh again");

        // A done plan never rots: give it a mention, finish it, churn away.
        fs::create_dir_all(src.path().join("docs/plans")).unwrap();
        fs::write(
            src.path().join("docs/plans/refactor.md"),
            "# Refactor\n\nRework src/main.rs.\n",
        )
        .unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        let plan = StableId::derive(&["plan", "twin/app", "refactor"]);
        {
            let index = fresh_index(&store);
            crate::lifecycle::set(
                &store,
                &index,
                &plan,
                crate::lifecycle::Lifecycle::Done,
                None,
            )
            .unwrap();
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
        fs::write(
            src.path().join("src/main.rs"),
            "pub fn main() { /* rewritten */ }\n",
        )
        .unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        let ins = insights(&store, "twin/app").unwrap();
        assert!(
            !ins.stale_docs.iter().any(|d| d.slug == "refactor"),
            "a finished plan is history, not rot: {:?}",
            ins.stale_docs
        );

        // rot=none on the kind's template exempts it entirely.
        crate::templates::seed(&store).unwrap();
        let tmpl = crate::templates::template_sid("adr");
        std::thread::sleep(std::time::Duration::from_millis(2));
        observe_src(&store, &tmpl, "rot", "none", "agent", now_ms()).unwrap();
        let ins = insights(&store, "twin/app").unwrap();
        assert!(
            !ins.stale_docs.iter().any(|d| d.kind == "decision"),
            "rot=none exempts the kind: {:?}",
            ins.stale_docs
        );
    }

    #[test]
    fn graph_defined_capture_rules_teach_new_artifact_kinds() {
        let (src, store_dir) = fixture();
        let store = Store::open(store_dir.path()).unwrap();
        crate::templates::seed(&store).unwrap();

        // Teach the store a "runbook" kind purely with observations.
        let tmpl = crate::templates::template_sid("runbook");
        store
            .put(&Object::Entity {
                id: tmpl.clone(),
                entity_kind: "template".to_string(),
                labels: BTreeMap::new(),
            })
            .unwrap();
        let now = now_ms();
        for (prop, value) in [
            ("applies_to", "runbook"),
            ("capture", "docs/runbooks/*.md"),
            ("fields", "title=heading, service=line"),
            ("requires", "title,service"),
        ] {
            observe_src(&store, &tmpl, prop, value, "agent", now).unwrap();
        }

        fs::create_dir_all(src.path().join("docs/runbooks")).unwrap();
        fs::write(
            src.path().join("docs/runbooks/deploy.md"),
            "# Deploy safely\n\nService: checkout\n\nRestart src/main.rs afterwards.\n",
        )
        .unwrap();
        fs::write(
            src.path().join("docs/runbooks/rollback.md"),
            "just some prose\n",
        )
        .unwrap();

        let r = refresh(&store, src.path(), "twin/app").unwrap();
        assert_eq!(r.docs.len(), 2, "both runbooks captured: {:?}", r.docs);

        let index = fresh_index(&store);
        let deploy = StableId::derive(&["runbook", "twin/app", "deploy"]);
        assert_eq!(
            latest(&index, &store, &deploy, "title").unwrap().as_deref(),
            Some("Deploy safely")
        );
        assert_eq!(
            latest(&index, &store, &deploy, "service")
                .unwrap()
                .as_deref(),
            Some("checkout"),
            "extracted field became an observation"
        );
        assert_eq!(
            latest(&index, &store, &deploy, "conforms")
                .unwrap()
                .as_deref(),
            Some("true")
        );
        // Mentions and concerns come from the shared core.
        let main_sid = StableId::derive(&["file", "src/main.rs"]);
        let mentioned: Vec<_> = index
            .relations_from(&deploy, "mentions")
            .iter()
            .filter_map(|id| match store.get(id) {
                Ok(Object::Relation { to, .. }) => Some(to),
                _ => None,
            })
            .collect();
        assert_eq!(mentioned, vec![main_sid.clone()]);
        assert_eq!(index.relations_from(&deploy, "concerns").len(), 1);

        // The prose-only runbook fails its contract — recorded, not rejected.
        let rollback = StableId::derive(&["runbook", "twin/app", "rollback"]);
        assert_eq!(
            latest(&index, &store, &rollback, "conforms")
                .unwrap()
                .as_deref(),
            Some("false")
        );
        let ins = insights(&store, "twin/app").unwrap();
        assert!(ins
            .nonconforming
            .iter()
            .any(|(s, k, m)| { s == "rollback" && k == "runbook" && m.contains("service") }));
        assert!(ins
            .custom_artifacts
            .iter()
            .any(|(k, n)| k == "runbook" && *n == 2));

        // Idempotence holds for rule-captured artifacts too.
        let before = store.count_objects().unwrap();
        let r2 = refresh(&store, src.path(), "twin/app").unwrap();
        assert!(r2.docs.is_empty());
        assert_eq!(store.count_objects().unwrap(), before, "no graph growth");

        // Staleness applies to the custom kind: the mentioned file changes.
        std::thread::sleep(std::time::Duration::from_millis(5));
        fs::write(
            src.path().join("src/main.rs"),
            "use crate::util;\npub fn main() { /* changed */ }\nstruct Config;\n",
        )
        .unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        let ins = insights(&store, "twin/app").unwrap();
        assert!(ins.stale_docs.iter().any(|d| {
            d.slug == "deploy"
                && d.kind == "runbook"
                && d.severity == Severity::Warn
                && d.changed.contains(&"src/main.rs".to_string())
        }));
    }

    #[test]
    fn rust_cross_crate_imports_resolve_to_sibling_crates() {
        let src = tempfile::tempdir().unwrap();
        fs::create_dir_all(src.path().join("crates/core-lib/src")).unwrap();
        fs::create_dir_all(src.path().join("crates/app/src")).unwrap();
        fs::write(
            src.path().join("crates/core-lib/src/lib.rs"),
            "pub mod ids;\n",
        )
        .unwrap();
        fs::write(
            src.path().join("crates/core-lib/src/ids.rs"),
            "pub struct Id;\n",
        )
        .unwrap();
        fs::write(
            src.path().join("crates/app/src/lib.rs"),
            "use core_lib::ids::Id;\nuse core_lib::helper;\npub fn go() {}\n",
        )
        .unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let store = Store::open(store_dir.path()).unwrap();
        refresh(&store, src.path(), "twin/ws").unwrap();

        let index = fresh_index(&store);
        let app = StableId::derive(&["file", "crates/app/src/lib.rs"]);
        let ids_rs = StableId::derive(&["file", "crates/core-lib/src/ids.rs"]);
        let core_root = StableId::derive(&["file", "crates/core-lib/src/lib.rs"]);
        let targets: Vec<_> = index
            .relations_from(&app, "imports")
            .iter()
            .filter_map(|id| match store.get(id) {
                Ok(Object::Relation { to, .. }) => Some(to),
                _ => None,
            })
            .collect();
        // `core_lib::ids::Id` -> the module file (hyphens matched from
        // underscores); `core_lib::helper` -> the crate root fallback.
        assert!(targets.contains(&ids_rs), "{targets:?}");
        assert!(targets.contains(&core_root), "{targets:?}");

        // A --full refresh after an extractor upgrade is guarded too:
        // reprocessing everything writes no duplicate facts.
        let before = store.count_objects().unwrap();
        refresh_full(&store, src.path(), "twin/ws").unwrap();
        assert_eq!(
            store.count_objects().unwrap(),
            before,
            "full reprocess, zero growth"
        );
    }

    #[test]
    fn files_at_reads_the_twin_as_it_was() {
        let (src, store_dir) = fixture();
        let store = Store::open(store_dir.path()).unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        let index = fresh_index(&store);
        let py = StableId::derive(&["file", "run.py"]);
        let (t1, old_hash) = latest_at(&index, &store, &py, "content_b3")
            .unwrap()
            .expect("first hash");

        // Later: run.py changes, new.rs appears.
        std::thread::sleep(std::time::Duration::from_millis(5));
        fs::write(
            src.path().join("run.py"),
            "import os\ndef main():\n    return 9\n",
        )
        .unwrap();
        fs::write(src.path().join("new.rs"), "pub fn fresh() {}\n").unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();

        let index = fresh_index(&store);
        // As of t1: the old hash, and new.rs does not exist yet.
        let then = files_at(&store, &index, "twin/app", t1).unwrap();
        let at_t1 = then
            .iter()
            .find(|(r, _)| r == "run.py")
            .expect("run.py existed");
        assert_eq!(at_t1.1, old_hash);
        assert!(!then.iter().any(|(r, _)| r == "new.rs"));
        // Now: the new hash, and new.rs is present.
        let now = files_at(&store, &index, "twin/app", u64::MAX).unwrap();
        let current = now.iter().find(|(r, _)| r == "run.py").unwrap();
        assert_ne!(current.1, old_hash);
        assert!(now.iter().any(|(r, _)| r == "new.rs"));
    }

    #[test]
    fn notes_attach_to_entities_and_survive_in_order() {
        let (src, store_dir) = fixture();
        let store = Store::open(store_dir.path()).unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();

        let sid = StableId::derive(&["file", "src/main.rs"]);
        add_note(&store, &sid, "entry point; config loading lives here").unwrap();
        add_note(&store, &sid, "Config struct is a stub").unwrap();

        let index = fresh_index(&store);
        let found = notes(&index, &store, &sid).unwrap();
        assert_eq!(found.len(), 2);
        assert!(found[0].1.contains("entry point"));
        assert!(found[1].1.contains("stub"));
    }

    #[test]
    fn vanished_structure_is_retracted_and_restored_edges_reuse_relations() {
        let src = tempfile::tempdir().unwrap();
        fs::create_dir_all(src.path().join("src")).unwrap();
        fs::write(
            src.path().join("src/main.rs"),
            "use crate::util;\npub fn main() {}\npub fn extra() {}\n",
        )
        .unwrap();
        fs::write(src.path().join("src/util.rs"), "pub fn helper() {}\n").unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let store = Store::open(store_dir.path()).unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();

        let main = StableId::derive(&["file", "src/main.rs"]);
        let util = StableId::derive(&["file", "src/util.rs"]);
        {
            let index = fresh_index(&store);
            assert_eq!(
                live_from(&index, &store, &main, "contains").unwrap().len(),
                2
            );
            assert_eq!(live_to(&index, &store, &util, "imports").unwrap().len(), 1);
        }

        // Drop one symbol and the import: the vanished structure is retracted.
        std::thread::sleep(std::time::Duration::from_millis(3));
        fs::write(src.path().join("src/main.rs"), "pub fn main() {}\n").unwrap();
        let r = refresh(&store, src.path(), "twin/app").unwrap();
        assert!(
            r.retracted >= 2,
            "symbol + import retracted: {}",
            r.retracted
        );
        let index = fresh_index(&store);
        assert_eq!(
            live_from(&index, &store, &main, "contains").unwrap().len(),
            1
        );
        assert!(live_to(&index, &store, &util, "imports")
            .unwrap()
            .is_empty());
        // The relation objects themselves remain — history is never destroyed.
        assert_eq!(index.relations_from(&main, "contains").len(), 2);
        // Insights count live structure only.
        let ins = insights_with(&store, &index, "twin/app").unwrap();
        assert_eq!(ins.symbols, 2, "main() + helper(), extra() gone");
        assert!(
            ins.hubs.is_empty(),
            "util.rs stopped being a hub: {:?}",
            ins.hubs
        );

        // Idempotence: retraction is a transition, not a repeated write.
        let before = store.count_objects().unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        assert_eq!(
            store.count_objects().unwrap(),
            before,
            "no growth on re-refresh"
        );

        // Re-adding the import restores the edge via the existing relation
        // object: one active=true observation, no duplicate relation.
        std::thread::sleep(std::time::Duration::from_millis(3));
        fs::write(
            src.path().join("src/main.rs"),
            "use crate::util;\npub fn main() {}\n",
        )
        .unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        let index = fresh_index(&store);
        assert_eq!(live_to(&index, &store, &util, "imports").unwrap().len(), 1);
        assert_eq!(
            index.relations_to(&util, "imports").len(),
            1,
            "no duplicate relation"
        );
        let before = store.count_objects().unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        assert_eq!(store.count_objects().unwrap(), before);
    }

    #[test]
    fn deleted_files_lose_their_edges_including_pre_tombstone_deletions() {
        let src = tempfile::tempdir().unwrap();
        fs::create_dir_all(src.path().join("src")).unwrap();
        fs::write(
            src.path().join("src/gone.rs"),
            "use crate::keep;\npub fn g() {}\n",
        )
        .unwrap();
        fs::write(src.path().join("src/keep.rs"), "pub fn k() {}\n").unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let store = Store::open(store_dir.path()).unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();

        fs::remove_file(src.path().join("src/gone.rs")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(3));
        let r = refresh(&store, src.path(), "twin/app").unwrap();
        assert_eq!(r.deleted, vec!["src/gone.rs".to_string()]);
        assert!(
            r.retracted >= 2,
            "contains + imports retracted: {}",
            r.retracted
        );
        let gone = StableId::derive(&["file", "src/gone.rs"]);
        let keep = StableId::derive(&["file", "src/keep.rs"]);
        let index = fresh_index(&store);
        assert!(live_from(&index, &store, &gone, "contains")
            .unwrap()
            .is_empty());
        assert!(live_to(&index, &store, &keep, "imports")
            .unwrap()
            .is_empty());

        // Healing: a live edge from an already-deleted file (as a store from
        // before tombstones would have) is retracted by the next refresh.
        let ghost = StableId::derive(&["symbol", "src/gone.rs", "fn", "ghost"]);
        store
            .put(&Object::Relation {
                from: gone.clone(),
                predicate: "contains".to_string(),
                to: ghost.clone(),
                source: "twin".to_string(),
                observed_at_ms: 1,
            })
            .unwrap();
        let r = refresh(&store, src.path(), "twin/app").unwrap();
        assert!(r.deleted.is_empty(), "already recorded as deleted");
        assert_eq!(r.retracted, 1, "the ghost edge is healed away");
        let index = fresh_index(&store);
        assert!(live_from(&index, &store, &gone, "contains")
            .unwrap()
            .is_empty());
        let before = store.count_objects().unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        assert_eq!(store.count_objects().unwrap(), before);
    }

    #[test]
    fn taught_extensions_ingest_only_where_their_globs_reach() {
        let src = tempfile::tempdir().unwrap();
        fs::create_dir_all(src.path().join("runs")).unwrap();
        fs::create_dir_all(src.path().join("stray")).unwrap();
        fs::write(src.path().join("runs/ledger.jsonl"), "{\"task\":\"t01\"}\n").unwrap();
        fs::write(src.path().join("stray/dump.jsonl"), "{}\n").unwrap();
        fs::write(src.path().join("notes.cfg"), "k=v\n").unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let store = Store::open(store_dir.path()).unwrap();

        // Teach a run-log kind whose extension only reaches runs/**.
        let tmpl = crate::templates::template_sid("run-log");
        store
            .put(&Object::Entity {
                id: tmpl.clone(),
                entity_kind: "template".to_string(),
                labels: BTreeMap::new(),
            })
            .unwrap();
        let now = now_ms();
        observe_src(&store, &tmpl, "applies_to", "run_log", "agent", now).unwrap();
        observe_src(&store, &tmpl, "capture", "runs/*.jsonl", "agent", now).unwrap();
        observe_src(&store, &tmpl, "extensions", "jsonl", "agent", now).unwrap();

        refresh(&store, src.path(), "twin/app").unwrap();
        let ns = store.namespace().unwrap();
        assert!(
            ns.contains_key("twin/app/runs/ledger.jsonl"),
            "in-glob jsonl ingested"
        );
        assert!(
            !ns.contains_key("twin/app/stray/dump.jsonl"),
            "stray jsonl invisible"
        );
        assert!(
            !ns.contains_key("twin/app/notes.cfg"),
            "untaught extension invisible"
        );
        // And it is captured as an artifact of the taught kind.
        let index = fresh_index(&store);
        let entity = StableId::derive(&["run_log", "twin/app", "ledger"]);
        assert!(latest(&index, &store, &entity, "content")
            .unwrap()
            .is_some());

        // Repo-level extensions apply everywhere (explicit opt-in).
        add_ingest_extensions(&store, "twin/app", &["cfg".to_string()]).unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        let ns = store.namespace().unwrap();
        assert!(ns.contains_key("twin/app/notes.cfg"));
    }

    #[test]
    fn compiled_kind_registry_captures_narrative_docs_and_stamps_contracts() {
        let src = tempfile::tempdir().unwrap();
        fs::create_dir_all(src.path().join("docs/adr")).unwrap();
        fs::create_dir_all(src.path().join("docs/runbooks")).unwrap();
        fs::write(
            src.path().join("README.md"),
            "# The Project\n\nStart at src/main.rs.\n",
        )
        .unwrap();
        fs::write(
            src.path().join("docs/architecture.md"),
            "# Architecture\n\nLayers.\n",
        )
        .unwrap();
        fs::write(
            src.path().join("docs/adr/adr-001-x.md"),
            "# X\n\nStatus: accepted\n",
        )
        .unwrap();
        fs::write(
            src.path().join("docs/runbooks/release.md"),
            "# Cutting a release\n\nService: brain\n",
        )
        .unwrap();
        fs::create_dir_all(src.path().join("src")).unwrap();
        fs::write(src.path().join("src/main.rs"), "pub fn main() {}\n").unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let store = Store::open(store_dir.path()).unwrap();
        // NO seed: compiled defaults alone must already capture.
        let r = refresh(&store, src.path(), "twin/app").unwrap();
        assert!(
            r.docs.len() >= 4,
            "README, architecture, adr, runbook: {:?}",
            r.docs
        );

        let index = fresh_index(&store);
        // README/docs/*.md become `doc` entities; the ADR path convention
        // keeps precedence (decision, not doc); runbook fields extract.
        let readme = StableId::derive(&["doc", "twin/app", "readme"]);
        assert_eq!(
            latest(&index, &store, &readme, "title").unwrap().as_deref(),
            Some("The Project")
        );
        assert_eq!(
            live_from(&index, &store, &readme, "mentions")
                .unwrap()
                .len(),
            1
        );
        let arch = StableId::derive(&["doc", "twin/app", "architecture"]);
        assert!(latest(&index, &store, &arch, "content").unwrap().is_some());
        let adr_as_doc = StableId::derive(&["doc", "twin/app", "adr-001-x"]);
        assert!(
            index.entity_nodes(&adr_as_doc).is_empty(),
            "builtin keeps the ADR"
        );
        let runbook = StableId::derive(&["runbook", "twin/app", "release"]);
        assert_eq!(
            latest(&index, &store, &runbook, "service")
                .unwrap()
                .as_deref(),
            Some("brain")
        );

        // README churn makes it stale at warn severity (narrative docs
        // describe the present).
        std::thread::sleep(std::time::Duration::from_millis(3));
        fs::write(
            src.path().join("src/main.rs"),
            "pub fn main() { /* v2 */ }\n",
        )
        .unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        let ins = insights(&store, "twin/app").unwrap();
        assert!(
            ins.stale_docs
                .iter()
                .any(|d| d.slug == "readme" && d.severity == Severity::Warn),
            "{:?}",
            ins.stale_docs
        );

        // After seeding, conformance runs and the judging contract version
        // is stamped on the artifact.
        crate::templates::seed(&store).unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        let index = fresh_index(&store);
        let stamped = latest(&index, &store, &readme, "template_b3")
            .unwrap()
            .unwrap();
        let tmpl = crate::templates::template_sid("doc");
        assert_eq!(
            Some(stamped),
            latest(&index, &store, &tmpl, "contract_b3").unwrap(),
            "artifact records the contract that judged it"
        );

        // Idempotence across the whole pipeline.
        let before = store.count_objects().unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        assert_eq!(store.count_objects().unwrap(), before);
    }

    #[test]
    fn same_run_moves_leave_a_renamed_to_trail() {
        let src = tempfile::tempdir().unwrap();
        fs::create_dir_all(src.path().join("src")).unwrap();
        fs::write(
            src.path().join("src/old.rs"),
            "pub fn stable_content() {}\n",
        )
        .unwrap();
        fs::write(src.path().join("src/twin_a.rs"), "pub fn dup() {}\n").unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let store = Store::open(store_dir.path()).unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();

        // Move: same bytes vanish here, appear there, in one refresh.
        fs::rename(src.path().join("src/old.rs"), src.path().join("src/new.rs")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(3));
        refresh(&store, src.path(), "twin/app").unwrap();
        let index = fresh_index(&store);
        let old = StableId::derive(&["file", "src/old.rs"]);
        let new = StableId::derive(&["file", "src/new.rs"]);
        assert_eq!(
            latest(&index, &store, &old, "present").unwrap().as_deref(),
            Some("false")
        );
        let trail = live_from(&index, &store, &old, "renamed_to").unwrap();
        assert_eq!(trail.len(), 1);
        assert_eq!(trail[0].1, new);

        // Ambiguous matches (two identical new files) leave no trail.
        fs::write(src.path().join("src/twin_b.rs"), "pub fn dup() {}\n").unwrap();
        fs::rename(
            src.path().join("src/twin_a.rs"),
            src.path().join("src/twin_c.rs"),
        )
        .unwrap();
        // twin_a's bytes now exist at BOTH twin_b and twin_c (new paths).
        fs::write(src.path().join("src/twin_c.rs"), "pub fn dup() {}\n").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(3));
        refresh(&store, src.path(), "twin/app").unwrap();
        let index = fresh_index(&store);
        let a = StableId::derive(&["file", "src/twin_a.rs"]);
        assert!(
            live_from(&index, &store, &a, "renamed_to")
                .unwrap()
                .is_empty(),
            "two candidates: no unique match, no trail"
        );

        // Idempotence still holds.
        let before = store.count_objects().unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        assert_eq!(store.count_objects().unwrap(), before);
    }

    #[test]
    fn dropped_mentions_retract_but_mentions_of_deleted_files_stay() {
        let src = tempfile::tempdir().unwrap();
        fs::create_dir_all(src.path().join("src")).unwrap();
        fs::create_dir_all(src.path().join("docs/adr")).unwrap();
        fs::write(src.path().join("src/a.rs"), "pub fn a() {}\n").unwrap();
        fs::write(src.path().join("src/b.rs"), "pub fn b() {}\n").unwrap();
        fs::write(
            src.path().join("docs/adr/adr-001-x.md"),
            "# X\n\nStatus: accepted\n\nAbout src/a.rs and src/b.rs.\n",
        )
        .unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let store = Store::open(store_dir.path()).unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();

        let doc = StableId::derive(&["decision", "twin/app", "adr-001-x"]);
        let a = StableId::derive(&["file", "src/a.rs"]);
        {
            let index = fresh_index(&store);
            assert_eq!(
                live_from(&index, &store, &doc, "mentions").unwrap().len(),
                2
            );
        }

        // The doc drops b.rs from its text: that mention is retracted, and
        // later churn in b.rs no longer makes the doc stale.
        std::thread::sleep(std::time::Duration::from_millis(3));
        fs::write(
            src.path().join("docs/adr/adr-001-x.md"),
            "# X\n\nStatus: accepted\n\nAbout src/a.rs only now.\n",
        )
        .unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        {
            let index = fresh_index(&store);
            let live: Vec<StableId> = live_from(&index, &store, &doc, "mentions")
                .unwrap()
                .into_iter()
                .map(|(_, to)| to)
                .collect();
            assert_eq!(live, vec![a.clone()]);
        }
        std::thread::sleep(std::time::Duration::from_millis(3));
        fs::write(src.path().join("src/b.rs"), "pub fn b() { /* churn */ }\n").unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        {
            let index = fresh_index(&store);
            let ins = insights_with(&store, &index, "twin/app").unwrap();
            assert!(
                ins.stale_docs.is_empty(),
                "b.rs churn is not the doc's problem: {:?}",
                ins.stale_docs
            );
        }

        // Deleting a.rs while the text still names it keeps the mention
        // live — that mismatch belongs to coherence, not retraction.
        fs::remove_file(src.path().join("src/a.rs")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(3));
        refresh(&store, src.path(), "twin/app").unwrap();
        // Touch the doc so it is re-recorded (the sweep re-runs).
        std::thread::sleep(std::time::Duration::from_millis(3));
        fs::write(
            src.path().join("docs/adr/adr-001-x.md"),
            "# X\n\nStatus: accepted\n\nAbout src/a.rs only now. Still.\n",
        )
        .unwrap();
        refresh(&store, src.path(), "twin/app").unwrap();
        let index = fresh_index(&store);
        let live: Vec<StableId> = live_from(&index, &store, &doc, "mentions")
            .unwrap()
            .into_iter()
            .map(|(_, to)| to)
            .collect();
        assert_eq!(
            live,
            vec![a],
            "mention of the deleted-but-still-named file stays"
        );
    }
}

#[cfg(test)]
mod note_order_tests {
    use super::*;
    use brain_index::replay;

    fn fresh_index(store: &Store) -> MemIndex {
        let mut index = MemIndex::new();
        replay(store, &mut index).unwrap();
        index
    }

    /// Notes come back in the order they were written, and the log — not
    /// the clock — is what says so. Two notes written in the same
    /// millisecond are indistinguishable by timestamp, so sorting by time
    /// would put them in an arbitrary order; the put feed knows which came
    /// first. `notes()` looks that position up rather than walking the
    /// whole log, and this is the invariant that makes the shortcut legal.
    #[test]
    fn notes_keep_their_true_order_even_within_one_millisecond() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let subject = StableId::derive(&["file", "src/lib.rs"]);
        let other = StableId::derive(&["file", "src/other.rs"]);

        // One frozen timestamp for every note: the clock cannot break ties.
        let at = 1_700_000_000_000u64;
        for text in ["first", "second", "third", "fourth"] {
            store
                .put(&Object::Observation {
                    subject: subject.clone(),
                    property: "note".to_string(),
                    value: text.to_string(),
                    source: "agent".to_string(),
                    observed_at_ms: at,
                })
                .unwrap();
            // Interleave another subject's writes, so position in the feed
            // is not the same as position among this subject's own notes.
            store
                .put(&Object::Observation {
                    subject: other.clone(),
                    property: "note".to_string(),
                    value: format!("{text}-elsewhere"),
                    source: "agent".to_string(),
                    observed_at_ms: at,
                })
                .unwrap();
        }
        // A non-note observation on the same subject must not appear.
        store
            .put(&Object::Observation {
                subject: subject.clone(),
                property: "present".to_string(),
                value: "true".to_string(),
                source: "twin".to_string(),
                observed_at_ms: at,
            })
            .unwrap();

        let index = fresh_index(&store);
        let got: Vec<String> = notes(&index, &store, &subject)
            .unwrap()
            .into_iter()
            .map(|(_, text)| text)
            .collect();
        assert_eq!(got, ["first", "second", "third", "fourth"]);

        let elsewhere: Vec<String> = notes(&index, &store, &other)
            .unwrap()
            .into_iter()
            .map(|(_, text)| text)
            .collect();
        assert_eq!(
            elsewhere,
            [
                "first-elsewhere",
                "second-elsewhere",
                "third-elsewhere",
                "fourth-elsewhere"
            ]
        );
    }

    /// The put feed is memoised behind the log's byte length. Appending
    /// must be visible immediately — a stale feed would hide new objects
    /// from replay, which is how the whole index is built.
    #[test]
    fn the_memoised_put_feed_sees_what_was_just_written() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let subject = StableId::derive(&["file", "a.rs"]);

        assert!(store.put_history().unwrap().is_empty());
        let first = store
            .put(&Object::Observation {
                subject: subject.clone(),
                property: "note".to_string(),
                value: "one".to_string(),
                source: "agent".to_string(),
                observed_at_ms: 1,
            })
            .unwrap();
        assert_eq!(store.put_history().unwrap(), vec![first]);
        assert_eq!(store.put_position().unwrap().get(&first), Some(&0));

        let second = store
            .put(&Object::Observation {
                subject,
                property: "note".to_string(),
                value: "two".to_string(),
                source: "agent".to_string(),
                observed_at_ms: 2,
            })
            .unwrap();
        assert_eq!(store.put_history().unwrap(), vec![first, second]);
        assert_eq!(store.put_position().unwrap().get(&second), Some(&1));
    }
}
